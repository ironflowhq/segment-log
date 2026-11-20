use super::*;
use keys::SealedSummary;
use std::{collections::HashMap, ops::RangeInclusive};

pub(super) struct View {
    pub storage: Arc<dyn Storage>,
    pub dir: PathBuf,
    pub log_id: codec::LogId,
    pub published: Mutex<Published>,
    pub tables: Mutex<Tables>,
}

#[derive(Default)]
pub(super) struct Published {
    pub watermark: u64,
    pub heads: BTreeMap<Vec<u8>, StreamHead>,
    pub sealed: Vec<Arc<SealedSegment>>,
    pub newest: Newest,
}

/// The segment that takes new frames.
#[derive(Default)]
pub(super) struct Newest {
    pub base: u64,
    /// The lowest epoch that a group frame here may carry: the highest epoch of the segment before,
    /// or this segment's header epoch if that is higher.
    pub epoch_floor: u64,
    pub spans: Vec<GroupSpan>,
}

pub(super) struct SealedSegment {
    pub base: u64,
    pub next: u64,
    groups_end: u64,
    epochs: RangeInclusive<u64>,
}

/// The bytes that the frame tables of sealed segments may hold in one view. At one kept frame per
/// `WALK_CHUNK`, this covers about 64 GiB of sealed history (ADR 0069).
pub(crate) const FRAME_TABLE_BUDGET: usize = 24 * 1024 * 1024;
/// What a kept table costs beside its entries: its shared allocation and the entries that find it.
const TABLE_OVERHEAD: usize = 64;

/// What a table of `entries` kept frames counts against the budget.
pub(crate) fn frame_table_charge(entries: usize) -> usize {
    TABLE_OVERHEAD + entries * std::mem::size_of::<GroupSpan>()
}

/// The sparse frame tables of sealed segments that reads built or rolls left, under a byte budget.
pub(super) struct Tables {
    budget: usize,
    charged: usize,
    uses: u64,
    /// Each table by its segment's base offset, with its last use.
    kept: HashMap<u64, (u64, Arc<[GroupSpan]>)>,
    /// The base offset of each kept table by its last use, least recent first.
    by_use: BTreeMap<u64, u64>,
}

impl Default for Tables {
    fn default() -> Self {
        Self {
            budget: FRAME_TABLE_BUDGET,
            charged: 0,
            uses: 0,
            kept: HashMap::new(),
            by_use: BTreeMap::new(),
        }
    }
}

impl Tables {
    #[cfg(test)]
    pub(super) fn with_budget(budget: usize) -> Self {
        Self {
            budget,
            ..Self::default()
        }
    }

    fn get(&mut self, base: u64) -> Option<Arc<[GroupSpan]>> {
        let (used, table) = self.kept.get_mut(&base)?;
        self.by_use.remove(used);
        self.uses += 1;
        *used = self.uses;
        self.by_use.insert(self.uses, base);
        Some(table.clone())
    }

    pub(super) fn keep(&mut self, base: u64, table: Arc<[GroupSpan]>) {
        self.uses += 1;
        self.charged += frame_table_charge(table.len());
        if let Some((used, old)) = self.kept.insert(base, (self.uses, table)) {
            self.by_use.remove(&used);
            self.charged -= frame_table_charge(old.len());
        }
        self.by_use.insert(self.uses, base);
        // The table just kept stays even alone over the budget, so the read that built it is the
        // only walk it costs.
        while self.charged > self.budget {
            let Some((&used, &oldest)) = self.by_use.first_key_value() else {
                break;
            };
            if used == self.uses {
                break;
            }
            self.by_use.remove(&used);
            let (_, old) = self.kept.remove(&oldest).expect("by_use names kept tables");
            self.charged -= frame_table_charge(old.len());
        }
    }
}

const WALK_CHUNK: u64 = 64 * 1024;
/// A frame envelope and its body header.
const PREFIX: usize = 8 + codec::BODY_HEADER_LEN;

/// Whether a group frame at `position` is kept: the first at or after a `WALK_CHUNK` boundary of
/// file position, given the position of the frame before it.
fn is_kept(previous: Option<u64>, position: u64) -> bool {
    previous.is_none_or(|before| position / WALK_CHUNK > before / WALK_CHUNK)
}

/// The kept frames of a segment's dense spans.
fn sparse(spans: &[GroupSpan]) -> Arc<[GroupSpan]> {
    let mut previous = None;
    spans
        .iter()
        .filter(|span| is_kept(previous.replace(span.position), span.position))
        .copied()
        .collect()
}

impl SealedSegment {
    /// The group frame whose envelope and body header are `prefix`, at `position`, if it follows
    /// offset `next` at an epoch from `floor` to the segment's highest, and ends by `limit`.
    fn group_at(
        &self,
        prefix: &[u8],
        position: u64,
        next: u64,
        floor: u64,
        limit: u64,
    ) -> Option<(GroupSpan, u64)> {
        let body = u32::from_le_bytes(prefix[0..4].try_into().unwrap()) as usize;
        let epoch = u64::from_le_bytes(prefix[12..20].try_into().unwrap());
        let base = u64::from_le_bytes(prefix[20..28].try_into().unwrap());
        let count = u32::from_le_bytes(prefix[28..32].try_into().unwrap());
        let valid = (codec::GROUP_HEADER_LEN..=codec::GROUP_BODY_LIMIT).contains(&body)
            && position + 8 + body as u64 <= limit
            && prefix[8] == 1
            && (floor..=*self.epochs.end()).contains(&epoch)
            && base == next
            && count > 0
            && base.checked_add(u64::from(count)).is_some();
        let span = GroupSpan {
            position,
            length: 8 + body as u32,
            base,
            count,
        };
        valid.then_some((span, epoch))
    }

    /// Walks every group frame's envelope and body header, and keeps the sparse table. `read`
    /// checks each frame's checksum when it decodes the frame.
    fn walk(&self, storage: &dyn Storage, dir: &Path) -> Result<Arc<[GroupSpan]>, ReadError> {
        let groups_end = self.groups_end;
        let path = segment_path(dir, self.base);
        let mut kept = vec![];
        let mut chunk: Vec<u8> = vec![];
        let mut chunk_at = codec::HEADER_LEN as u64;
        let mut position = chunk_at;
        let mut previous = None;
        let mut next = self.base;
        let mut floor = *self.epochs.start();
        while position < groups_end {
            let corrupt = || ReadError::Corrupt {
                segment: self.base,
                position,
            };
            if position + PREFIX as u64 > chunk_at + chunk.len() as u64 {
                let length = (groups_end - position).min(WALK_CHUNK) as usize;
                chunk = read_bytes(storage, &path, Purpose::Lookup, position, length)
                    .map_err(ReadError::Io)?;
                chunk_at = position;
                if chunk.len() != length || length < PREFIX {
                    return Err(corrupt());
                }
            }
            let at = (position - chunk_at) as usize;
            let (span, epoch) = self
                .group_at(&chunk[at..at + PREFIX], position, next, floor, groups_end)
                .ok_or_else(corrupt)?;
            if is_kept(previous.replace(position), position) {
                kept.push(span);
            }
            (next, floor) = (span.end(), epoch);
            position += u64::from(span.length);
        }
        if next != self.next {
            return Err(ReadError::Corrupt {
                segment: self.base,
                position,
            });
        }
        Ok(kept.into())
    }

    /// The group frames after kept frame `kept` and before the next kept frame, at position and
    /// base offset `after`, or the end of the groups. Every one of them starts before the boundary
    /// after `kept`'s, so one read from the end of `kept` to at most `WALK_CHUNK` and one header
    /// past its start holds their headers. Returns that read's bytes, where it starts, and the
    /// frames.
    fn window(
        &self,
        storage: &dyn Storage,
        dir: &Path,
        kept: &GroupSpan,
        (end, after): (u64, u64),
    ) -> Result<(Vec<u8>, u64, Vec<GroupSpan>), ReadError> {
        let start = kept.position + u64::from(kept.length);
        let cut = end.min(kept.position + WALK_CHUNK + PREFIX as u64);
        let mut bytes = vec![];
        if cut > start {
            let length = (cut - start) as usize;
            let path = segment_path(dir, self.base);
            bytes = read_bytes(storage, &path, Purpose::Lookup, start, length)
                .map_err(ReadError::Io)?;
            if bytes.len() != length {
                return Err(ReadError::Corrupt {
                    segment: self.base,
                    position: start,
                });
            }
        }
        let mut spans = vec![];
        let mut position = start;
        // No epoch is kept, so the floor is the segment's lowest, not `kept`'s.
        let (mut next, mut floor) = (kept.end(), *self.epochs.start());
        while position < end {
            let corrupt = ReadError::Corrupt {
                segment: self.base,
                position,
            };
            let at = (position - start) as usize;
            let Some(prefix) = bytes.get(at..at + PREFIX) else {
                return Err(corrupt);
            };
            let Some((span, epoch)) = self.group_at(prefix, position, next, floor, end) else {
                return Err(corrupt);
            };
            (next, floor) = (span.end(), epoch);
            position += u64::from(span.length);
            spans.push(span);
        }
        if next != after {
            return Err(ReadError::Corrupt {
                segment: self.base,
                position,
            });
        }
        Ok((bytes, start, spans))
    }
}

#[derive(Clone, Copy)]
pub(super) struct GroupSpan {
    pub position: u64,
    pub length: u32,
    pub base: u64,
    pub count: u32,
}

impl GroupSpan {
    pub(super) fn of(position: u64, length: usize, group: &codec::Group<'_>) -> Self {
        Self {
            position,
            length: u32::try_from(length).expect("a frame is under the body limit"),
            base: group.base_offset,
            count: u32::try_from(group.entries.len()).expect("a group is under the body limit"),
        }
    }

    pub(super) fn end(&self) -> u64 {
        self.base + u64::from(self.count)
    }
}

impl Published {
    pub(super) fn publish(&mut self, span: GroupSpan, group: &codec::Group<'_>) {
        for (i, entry) in group.entries.iter().enumerate() {
            self.heads.insert(
                entry.stream.to_vec(),
                StreamHead {
                    revision: entry.revision,
                    offset: group.base_offset + i as u64,
                },
            );
        }
        self.watermark = span.end();
        self.newest.spans.push(span);
    }

    /// Seals the newest segment, and returns its sparse table.
    pub(super) fn roll(&mut self, epoch: u64, next_epoch: u64) -> Arc<[GroupSpan]> {
        let spans = std::mem::take(&mut self.newest.spans);
        let groups_end = spans.last().map_or(codec::HEADER_LEN as u64, |last| {
            last.position + u64::from(last.length)
        });
        self.sealed.push(Arc::new(SealedSegment {
            base: self.newest.base,
            next: self.watermark,
            groups_end,
            epochs: self.newest.epoch_floor..=epoch,
        }));
        self.newest.base = self.watermark;
        self.newest.epoch_floor = next_epoch;
        sparse(&spans)
    }

    pub(super) fn opened(
        &mut self,
        summary: &SealedSummary,
        footer: &codec::Footer<'_>,
        epochs: RangeInclusive<u64>,
    ) {
        for tail in &footer.streams {
            let head = StreamHead {
                revision: tail.last_revision,
                offset: tail.last_offset,
            };
            match self.heads.get_mut(tail.stream) {
                Some(known) => *known = head,
                None => {
                    self.heads.insert(tail.stream.to_vec(), head);
                }
            }
        }
        self.sealed.push(Arc::new(SealedSegment {
            base: summary.base,
            next: summary.next,
            groups_end: summary.filter_at,
            epochs,
        }));
        self.watermark = summary.next;
        self.newest.base = summary.next;
    }
}

fn overlapping(spans: &[GroupSpan], from: u64, end: u64) -> &[GroupSpan] {
    let first = spans.partition_point(|span| span.end() <= from);
    let last = spans.partition_point(|span| span.base < end);
    &spans[first..last]
}

impl View {
    pub(super) fn watermark(&self) -> LogOffset {
        self.published.lock().unwrap().watermark
    }

    pub(super) fn stream_head(&self, stream: &[u8]) -> Option<StreamHead> {
        self.published
            .lock()
            .unwrap()
            .heads
            .get(stream)
            .map(|h| StreamHead {
                revision: h.revision,
                offset: h.offset,
            })
    }

    pub(super) fn read(&self, from: LogOffset, max: usize) -> Result<Vec<Record>, ReadError> {
        let end = from.saturating_add(max as u64);
        let (sealed, newest_base, newest) = {
            let state = self.published.lock().unwrap();
            if max == 0 || from >= state.watermark {
                return Ok(vec![]);
            }
            let first = state.sealed.partition_point(|s| s.next <= from);
            let last = state.sealed.partition_point(|s| s.base < end);
            (
                state.sealed[first..last].to_vec(),
                state.newest.base,
                overlapping(&state.newest.spans, from, end).to_vec(),
            )
        };
        let mut records = Vec::new();
        for segment in &sealed {
            if self.read_sealed(segment, from, max, &mut records)? {
                return Ok(records);
            }
        }
        for span in &newest {
            if self.collect(newest_base, span, from, max, &mut records)? {
                return Ok(records);
            }
        }
        Ok(records)
    }

    fn spans(&self, segment: &SealedSegment) -> Result<Arc<[GroupSpan]>, ReadError> {
        if let Some(spans) = self.tables.lock().unwrap().get(segment.base) {
            return Ok(spans);
        }
        // The table is a function of sealed bytes, so two readers that both walk build the same
        // table.
        let spans = segment.walk(self.storage.as_ref(), &self.dir)?;
        self.tables
            .lock()
            .unwrap()
            .keep(segment.base, spans.clone());
        Ok(spans)
    }

    /// Reads a sealed segment's records from `from` through its kept frames and the windows
    /// between them, and returns whether `records` holds `max`.
    fn read_sealed(
        &self,
        segment: &SealedSegment,
        from: u64,
        max: usize,
        records: &mut Vec<Record>,
    ) -> Result<bool, ReadError> {
        let kept = self.spans(segment)?;
        let first = kept.partition_point(|k| k.base <= from).saturating_sub(1);
        for (i, k) in kept.iter().enumerate().skip(first) {
            if k.end() > from && self.collect(segment.base, k, from, max, records)? {
                return Ok(true);
            }
            let after = kept
                .get(i + 1)
                .map_or((segment.groups_end, segment.next), |n| (n.position, n.base));
            if after.1 <= from {
                continue;
            }
            let (bytes, at, spans) = segment.window(self.storage.as_ref(), &self.dir, k, after)?;
            for span in spans.iter().filter(|span| span.end() > from) {
                let inside = (span.position - at) as usize;
                let done = match bytes.get(inside..inside + span.length as usize) {
                    Some(frame) => self.decode(segment.base, span, frame, from, max, records)?,
                    None => self.collect(segment.base, span, from, max, records)?,
                };
                if done {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn collect(
        &self,
        segment: u64,
        span: &GroupSpan,
        from: u64,
        max: usize,
        records: &mut Vec<Record>,
    ) -> Result<bool, ReadError> {
        let bytes = read_bytes(
            self.storage.as_ref(),
            &segment_path(&self.dir, segment),
            Purpose::Lookup,
            span.position,
            span.length as usize,
        )
        .map_err(ReadError::Io)?;
        self.decode(segment, span, &bytes, from, max, records)
    }

    fn decode(
        &self,
        segment: u64,
        span: &GroupSpan,
        bytes: &[u8],
        from: u64,
        max: usize,
        records: &mut Vec<Record>,
    ) -> Result<bool, ReadError> {
        let corrupt = || ReadError::Corrupt {
            segment,
            position: span.position,
        };
        let group = codec::decode(&self.log_id, bytes)
            .ok()
            .and_then(codec::Frame::into_group)
            .ok_or_else(corrupt)?;
        if group.base_offset != span.base || group.entries.len() != span.count as usize {
            return Err(corrupt());
        }
        for (i, entry) in group.entries.into_iter().enumerate() {
            let offset = group.base_offset + i as u64;
            if offset >= from {
                records.push(Record {
                    offset,
                    stream: entry.stream.to_vec(),
                    revision: entry.revision,
                    key: entry.key.map(<[u8]>::to_vec),
                    payload: entry.payload.to_vec(),
                });
                if records.len() == max {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
