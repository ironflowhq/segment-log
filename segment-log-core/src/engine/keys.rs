use super::*;

enum KeyLocation {
    Durable {
        frame: Frame,
        entry: usize,
        position: Position,
    },
    Pending {
        request: usize,
        record: usize,
        position: Position,
    },
}

#[derive(Default)]
pub(super) struct Keys(BTreeMap<Vec<u8>, KeyLocation>);

pub(super) enum Classification {
    New,
    DurableRetry(Vec<Position>),
    PendingRetry(Vec<Position>),
    Rejected(CommitError),
    LookupFailure,
}

impl Keys {
    pub(super) fn publish(&mut self, frame: &Frame, group: &codec::Group<'_>) {
        for (entry, record) in group.entries.iter().enumerate() {
            if let Some(key) = record.key {
                self.0.insert(
                    key.to_vec(),
                    KeyLocation::Durable {
                        frame: frame.clone(),
                        entry,
                        position: Position {
                            offset: group.base_offset + entry as u64,
                            revision: record.revision,
                        },
                    },
                );
            }
        }
    }

    pub(super) fn stage(
        &mut self,
        request: usize,
        submit: &Submit,
        base: u64,
        revisions: &[RevisionSlot],
    ) {
        for (record, (input, revision)) in submit.records.iter().zip(revisions).enumerate() {
            if let Some(key) = &input.key {
                self.0.insert(
                    key.clone(),
                    KeyLocation::Pending {
                        request,
                        record,
                        position: Position {
                            offset: base + record as u64,
                            revision: revision.revision,
                        },
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
        pending: &[Request],
    ) -> Classification {
        let mut positions = Vec::new();
        let mut has_pending = false;
        for (index, record) in submit.records.iter().enumerate() {
            let Some(original) = record.key.as_ref().and_then(|key| self.0.get(key)) else {
                continue;
            };
            let position = match original {
                KeyLocation::Pending {
                    request,
                    record: record_index,
                    position,
                } => {
                    let entry = &pending[*request].submit.records[*record_index];
                    if entry.stream != record.stream || entry.payload != record.payload {
                        return Classification::Rejected(CommitError::KeyConflict { index });
                    }
                    has_pending = true;
                    position
                }
                KeyLocation::Durable {
                    frame,
                    entry: entry_index,
                    position,
                } => {
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
                        || group.base_offset.checked_add(group.entries.len() as u64)
                            != Some(frame.end)
                    {
                        return Classification::LookupFailure;
                    }
                    let Some(entry) = group.entries.get(*entry_index) else {
                        return Classification::LookupFailure;
                    };
                    if entry.key != record.key.as_deref()
                        || entry.revision != position.revision
                        || group.base_offset + *entry_index as u64 != position.offset
                    {
                        return Classification::LookupFailure;
                    }
                    if entry.stream != record.stream || entry.payload != record.payload {
                        return Classification::Rejected(CommitError::KeyConflict { index });
                    }
                    position
                }
            };
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
