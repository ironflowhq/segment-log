use super::*;
use keys::SealedSummary;
use std::{ops::RangeInclusive, sync::OnceLock};

pub(super) struct View {
    pub storage: Arc<dyn Storage>,
    pub dir: PathBuf,
    pub log_id: codec::LogId,
    pub published: Mutex<Published>,
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
    pub spans: Vec<GroupSpan>,
}

pub(super) struct SealedSegment {
    pub base: u64,
    pub next: u64,
    spans: Spans,
}

enum Spans {
    Rolled(Arc<[GroupSpan]>),
    /// The table is a function of sealed bytes, so a second reader that builds it at the same time
    /// is harmless.
    Lazy {
        groups_end: u64,
        epochs: RangeInclusive<u64>,
        built: OnceLock<Arc<[GroupSpan]>>,
    },
}

const WALK_CHUNK: u64 = 64 * 1024;

impl SealedSegment {
    fn spans(&self, storage: &dyn Storage, dir: &Path) -> Result<Arc<[GroupSpan]>, ReadError> {
        match &self.spans {
            Spans::Rolled(spans) => Ok(spans.clone()),
            Spans::Lazy {
                groups_end,
                epochs,
                built,
            } => {
                if let Some(spans) = built.get() {
                    return Ok(spans.clone());
                }
                let spans = self.walk(storage, dir, *groups_end, epochs)?;
                Ok(built.get_or_init(|| spans).clone())
            }
        }
    }

    /// `read` checks each frame's checksum when it decodes the frame.
    fn walk(
        &self,
        storage: &dyn Storage,
        dir: &Path,
        groups_end: u64,
        epochs: &RangeInclusive<u64>,
    ) -> Result<Arc<[GroupSpan]>, ReadError> {
        const PREFIX: usize = 8 + codec::BODY_HEADER_LEN;
        let path = segment_path(dir, self.base);
        let mut spans = vec![];
        let mut chunk: Vec<u8> = vec![];
        let mut chunk_at = codec::HEADER_LEN as u64;
        let mut position = chunk_at;
        let mut next = self.base;
        let mut floor = *epochs.start();
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
            let prefix = &chunk[at..at + PREFIX];
            let body = u32::from_le_bytes(prefix[0..4].try_into().unwrap()) as usize;
            let epoch = u64::from_le_bytes(prefix[12..20].try_into().unwrap());
            let base = u64::from_le_bytes(prefix[20..28].try_into().unwrap());
            let count = u32::from_le_bytes(prefix[28..32].try_into().unwrap());
            let end = position + 8 + body as u64;
            let valid = (codec::BODY_HEADER_LEN..=codec::GROUP_BODY_LIMIT).contains(&body)
                && end <= groups_end
                && prefix[8] == 1
                && (floor..=*epochs.end()).contains(&epoch)
                && base == next
                && count > 0;
            next = match base.checked_add(u64::from(count)) {
                Some(after) if valid => after,
                _ => return Err(corrupt()),
            };
            floor = epoch;
            spans.push(GroupSpan {
                position,
                length: 8 + body as u32,
                base,
                count,
            });
            position = end;
        }
        if next != self.next {
            return Err(ReadError::Corrupt {
                segment: self.base,
                position,
            });
        }
        Ok(spans.into())
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

    pub(super) fn roll(&mut self) {
        let spans = std::mem::take(&mut self.newest.spans).into();
        self.sealed.push(Arc::new(SealedSegment {
            base: self.newest.base,
            next: self.watermark,
            spans: Spans::Rolled(spans),
        }));
        self.newest.base = self.watermark;
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
            spans: Spans::Lazy {
                groups_end: summary.filter_at,
                epochs,
                built: OnceLock::new(),
            },
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
            let spans = segment.spans(self.storage.as_ref(), &self.dir)?;
            for span in overlapping(&spans, from, end) {
                if self.collect(segment.base, span, from, max, &mut records)? {
                    return Ok(records);
                }
            }
        }
        for span in &newest {
            if self.collect(newest_base, span, from, max, &mut records)? {
                return Ok(records);
            }
        }
        Ok(records)
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
        let corrupt = || ReadError::Corrupt {
            segment,
            position: span.position,
        };
        let group = codec::decode(&self.log_id, &bytes)
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
