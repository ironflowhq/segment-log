use super::*;

struct DurableKey {
    frame: Frame,
    entry: usize,
    position: Position,
}

#[derive(Default)]
pub(super) struct DurableKeys(BTreeMap<Vec<u8>, DurableKey>);

pub(super) enum Classification {
    New,
    Retry(Vec<Position>),
    Rejected(CommitError),
    LookupFailure,
}

impl DurableKeys {
    pub(super) fn publish(&mut self, frame: &Frame, group: &codec::Group<'_>) {
        for (entry, record) in group.entries.iter().enumerate() {
            if let Some(key) = record.key {
                self.0.insert(
                    key.to_vec(),
                    DurableKey {
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

    pub(super) fn classify(
        &self,
        storage: &dyn Storage,
        segment: &Path,
        submit: &Submit,
        scratch: &mut Vec<u8>,
    ) -> Classification {
        let mut positions = Vec::new();
        for (index, record) in submit.records.iter().enumerate() {
            let Some(original) = record.key.as_ref().and_then(|key| self.0.get(key)) else {
                continue;
            };
            let frame = &original.frame;
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
            let Some(entry) = group.entries.get(original.entry) else {
                return Classification::LookupFailure;
            };
            if entry.key != record.key.as_deref()
                || entry.revision != original.position.revision
                || group.base_offset + original.entry as u64 != original.position.offset
            {
                return Classification::LookupFailure;
            }
            if entry.stream != record.stream || entry.payload != record.payload {
                return Classification::Rejected(CommitError::KeyConflict { index });
            }
            if positions.is_empty() {
                positions = Vec::with_capacity(submit.records.len());
            }
            positions.push(Position {
                offset: original.position.offset,
                revision: entry.revision,
            });
        }
        if positions.is_empty() {
            Classification::New
        } else if positions.len() == submit.records.len() {
            Classification::Retry(positions)
        } else {
            Classification::Rejected(CommitError::PartialRetry)
        }
    }
}
