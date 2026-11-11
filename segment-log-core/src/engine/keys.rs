use super::*;

/// Durable keys and the keys staged by the current group. A staged position is
/// final: its group becomes durable or the writer stops.
pub(super) struct Keys {
    log: codec::LogId,
    map: BTreeMap<Vec<u8>, Position>,
}

/// The current group's new records. An offset at or past `base` is still pending.
pub(super) struct Staged<'a> {
    pub base: u64,
    pub requests: &'a [Request],
    /// Request and record index of each staged record, by offset past `base`.
    pub origins: &'a [(usize, usize)],
}

pub(super) enum Classification {
    New,
    DurableRetry(Vec<Position>),
    PendingRetry(Vec<Position>),
    Rejected(CommitError),
    LookupFailure,
}

impl Keys {
    pub(super) fn new(log: codec::LogId) -> Self {
        Self {
            log,
            map: BTreeMap::new(),
        }
    }

    pub(super) fn recover(&mut self, group: &codec::Group<'_>) {
        for (entry, record) in group.entries.iter().enumerate() {
            if let Some(key) = record.key {
                self.map.insert(
                    key.to_vec(),
                    Position {
                        offset: group.base_offset + entry as u64,
                        revision: record.revision,
                    },
                );
            }
        }
    }

    /// The key index entries of the segment whose records start at `base` and whose group frames
    /// are `spans`.
    pub(super) fn index_entries(&self, base: u64, spans: &[GroupSpan]) -> Vec<codec::KeyEntry> {
        self.map
            .iter()
            .filter(|(_, position)| position.offset >= base)
            .map(|(key, position)| {
                let span = spans[spans.partition_point(|span| span.end() <= position.offset)];
                codec::KeyEntry {
                    hash: self.log.key_hash(key),
                    position: span.position as u32,
                    record: (position.offset - span.base) as u32,
                }
            })
            .collect()
    }

    pub(super) fn stage(&mut self, submit: &Submit, base: u64, revisions: &[RevisionSlot]) {
        for (record, (input, revision)) in submit.records.iter().zip(revisions).enumerate() {
            if let Some(key) = &input.key {
                self.map.insert(
                    key.clone(),
                    Position {
                        offset: base + record as u64,
                        revision: revision.revision,
                    },
                );
            }
        }
    }

    pub(super) fn classify(
        &self,
        view: &View,
        submit: &Submit,
        scratch: &mut Vec<u8>,
        staged: &Staged<'_>,
    ) -> Classification {
        let mut positions = Vec::new();
        let mut has_pending = false;
        for (index, record) in submit.records.iter().enumerate() {
            let Some(position) = record.key.as_ref().and_then(|key| self.map.get(key)) else {
                continue;
            };
            if position.offset >= staged.base {
                let (request, original) = staged.origins[(position.offset - staged.base) as usize];
                let entry = &staged.requests[request].submit.records[original];
                if entry.stream != record.stream || entry.payload != record.payload {
                    return Classification::Rejected(CommitError::KeyConflict { index });
                }
                has_pending = true;
            } else {
                let span = view.published.lock().unwrap().span_at(position.offset);
                let Some((segment, span)) = span else {
                    return Classification::LookupFailure;
                };
                let length = span.length as usize;
                let entry_index = (position.offset - span.base) as usize;
                let Ok(bytes) = read_bytes(
                    view.storage.as_ref(),
                    &segment_path(&view.dir, segment),
                    Purpose::Lookup,
                    span.position,
                    length,
                ) else {
                    return Classification::LookupFailure;
                };
                if bytes.len() != length {
                    return Classification::LookupFailure;
                }
                *scratch = bytes;
                let Some(group) = codec::decode(&self.log, scratch)
                    .ok()
                    .and_then(codec::Frame::into_group)
                else {
                    return Classification::LookupFailure;
                };
                if group.base_offset != span.base || group.entries.len() != span.count as usize {
                    return Classification::LookupFailure;
                }
                let Some(entry) = group.entries.get(entry_index) else {
                    return Classification::LookupFailure;
                };
                if entry.key != record.key.as_deref() || entry.revision != position.revision {
                    return Classification::LookupFailure;
                }
                if entry.stream != record.stream || entry.payload != record.payload {
                    return Classification::Rejected(CommitError::KeyConflict { index });
                }
            }
            if positions.is_empty() {
                positions = Vec::with_capacity(submit.records.len());
            }
            positions.push(Position {
                offset: position.offset,
                revision: position.revision,
            });
        }
        if positions.is_empty() {
            Classification::New
        } else if positions.len() == submit.records.len() {
            if has_pending {
                Classification::PendingRetry(positions)
            } else {
                Classification::DurableRetry(positions)
            }
        } else {
            Classification::Rejected(CommitError::PartialRetry)
        }
    }
}
