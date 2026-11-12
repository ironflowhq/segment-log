use super::*;
use std::sync::OnceLock;

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
    spans: OnceLock<Arc<[GroupSpan]>>,
}

impl SealedSegment {
    pub(super) fn spans(&self) -> &[GroupSpan] {
        self.spans.get().expect("open fills every span table")
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
        let spans: Arc<[GroupSpan]> = std::mem::take(&mut self.newest.spans).into();
        self.sealed.push(Arc::new(SealedSegment {
            base: self.newest.base,
            next: self.watermark,
            spans: OnceLock::from(spans),
        }));
        self.newest.base = self.watermark;
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
        let spans = sealed
            .iter()
            .flat_map(|s| {
                overlapping(s.spans(), from, end)
                    .iter()
                    .map(|span| (s.base, *span))
            })
            .chain(newest.into_iter().map(|span| (newest_base, span)));
        let mut records = Vec::new();
        for (segment, span) in spans {
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
                        return Ok(records);
                    }
                }
            }
        }
        Ok(records)
    }
}
