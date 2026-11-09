use super::*;

/// Durable keys and the keys staged by the current group. A staged position is
/// final: its group becomes durable or the writer stops.
#[derive(Default)]
pub(super) struct Keys(BTreeMap<Vec<u8>, Position>);

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
    pub(super) fn recover(&mut self, group: &codec::Group<'_>) {
        for (entry, record) in group.entries.iter().enumerate() {
            if let Some(key) = record.key {
                self.0.insert(
                    key.to_vec(),
                    Position {
                        offset: group.base_offset + entry as u64,
                        revision: record.revision,
                    },
                );
            }
        }
    }

    pub(super) fn stage(&mut self, submit: &Submit, base: u64, revisions: &[RevisionSlot]) {
        for (record, (input, revision)) in submit.records.iter().zip(revisions).enumerate() {
            if let Some(key) = &input.key {
                self.0.insert(
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
        storage: &dyn Storage,
        segment: &Path,
        submit: &Submit,
        scratch: &mut Vec<u8>,
        staged: &Staged<'_>,
        published: &Mutex<Published>,
    ) -> Classification {
        let mut positions = Vec::new();
        let mut has_pending = false;
        for (index, record) in submit.records.iter().enumerate() {
            let Some(position) = record.key.as_ref().and_then(|key| self.0.get(key)) else {
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
                let frame = {
                    let durable = published.lock().unwrap();
                    let covering = durable
                        .frames
                        .partition_point(|frame| frame.end <= position.offset);
                    durable.frames[covering].clone()
                };
                let entry_index = (position.offset - frame.base) as usize;
                let Ok(bytes) = read_bytes(
                    storage,
                    segment,
                    Purpose::Lookup,
                    frame.position,
                    frame.length,
                ) else {
                    return Classification::LookupFailure;
                };
                if bytes.len() != frame.length {
                    return Classification::LookupFailure;
                }
                *scratch = bytes;
                let Ok(group) = codec::decode(scratch) else {
                    return Classification::LookupFailure;
                };
                if group.epoch != frame.epoch
                    || group.base_offset != frame.base
                    || group.base_offset.checked_add(group.entries.len() as u64) != Some(frame.end)
                {
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
