use super::*;
use closing::{RunState, Step, Tail};
use keys::{ActiveKey, SealedSummary};

pub(super) use read_only::ReadOnly;

mod read_only {
    use super::super::*;

    pub(in crate::engine) struct ReadOnly<'a>(&'a dyn Storage);

    impl<'a> ReadOnly<'a> {
        pub(in crate::engine) fn new(storage: &'a dyn Storage) -> Self {
            Self(storage)
        }

        pub(in crate::engine) fn read(
            &self,
            path: &Path,
            purpose: Purpose,
            position: u64,
            length: usize,
        ) -> io::Result<Vec<u8>> {
            read_bytes(self.0, path, purpose, position, length)
        }

        pub(super) fn length(&self, path: &Path, purpose: Purpose) -> io::Result<u64> {
            match call(self.0, path, purpose, Op::Open)? {
                Value::Length(length) => Ok(length),
                _ => Err(io::Error::other("storage did not return length")),
            }
        }

        pub(super) fn list(&self, dir: &Path, purpose: Purpose) -> io::Result<Vec<PathBuf>> {
            match call(self.0, dir, purpose, Op::List)? {
                Value::Names(names) => Ok(names),
                _ => Err(io::Error::other("storage did not list directory")),
            }
        }

        pub(in crate::engine) fn frame(
            &self,
            path: &Path,
            purpose: Purpose,
            position: u64,
            limit: u64,
        ) -> io::Result<Option<Vec<u8>>> {
            let envelope = self.read(path, purpose, position, 8)?;
            let Ok(envelope) = <[u8; 8]>::try_from(envelope) else {
                return Ok(None);
            };
            let body = u32::from_le_bytes(envelope[..4].try_into().unwrap()) as usize;
            let length = (body + 8) as u64;
            if !(codec::BODY_HEADER_LEN..=codec::FRAME_BODY_LIMIT).contains(&body)
                || position.checked_add(length).is_none_or(|end| end > limit)
            {
                return Ok(None);
            }
            let frame = self.read(path, purpose, position, body + 8)?;
            Ok((frame.len() == body + 8).then_some(frame))
        }
    }
}

pub(super) struct Survey {
    pub log_id: codec::LogId,
    pub published: Published,
    pub sealed: Vec<SealedSummary>,
    pub epoch: u64,
    pub newest_header: Vec<u8>,
    pub newest: Scan,
}

pub(super) struct Scan {
    pub end: u64,
    pub ending: Ending,
    pub last: Option<(u64, Vec<u8>)>,
    pub tails: BTreeMap<Vec<u8>, Tail>,
    pub keys: BTreeMap<Vec<u8>, ActiveKey>,
}

pub(super) enum Ending {
    Groups,
    Stopped,
    Rolled(Run),
}

pub(super) struct Run {
    pub summary: SealedSummary,
    pub frames: Vec<Vec<u8>>,
}

/// What the scan has kept so far, across the segments it has read.
struct Kept {
    published: Published,
    /// The epoch that the next frame must not be lower than.
    epoch: u64,
}

pub(super) fn segment_bases(storage: &ReadOnly<'_>, dir: &Path) -> io::Result<Vec<u64>> {
    let mut bases: Vec<u64> = storage
        .list(dir, Purpose::Open)?
        .iter()
        .filter_map(|name| {
            let digits = name.to_str()?.strip_suffix(".seg")?;
            if digits.len() == 20 && digits.bytes().all(|b| b.is_ascii_digit()) {
                digits.parse().ok()
            } else {
                None
            }
        })
        .collect();
    bases.sort_unstable();
    Ok(bases)
}

pub(super) fn is_torn_header(storage: &ReadOnly<'_>, dir: &Path, base: u64) -> io::Result<bool> {
    let path = segment_path(dir, base);
    let length = storage.length(&path, Purpose::Open)?;
    let header = storage.read(&path, Purpose::Open, 0, codec::HEADER_LEN)?;
    let crc_valid = header.len() == codec::HEADER_LEN
        && codec::crc32c(&header[..60]) == u32::from_le_bytes(header[60..].try_into().unwrap());
    Ok(length <= codec::HEADER_LEN as u64 && !crc_valid)
}

pub(super) fn survey(
    storage: &ReadOnly<'_>,
    dir: &Path,
    bases: &[u64],
) -> Result<Survey, OpenError> {
    let headers = bases
        .iter()
        .map(|&base| header(storage, dir, base))
        .collect::<Result<Vec<_>, _>>()?;
    let log_id = codec::LogId::new(headers[0].0.log_id);
    for (&base, (header, _)) in bases.iter().zip(&headers) {
        if header.log_id != *log_id.bytes() {
            return Err(OpenError::LogIdMismatch { segment: base });
        }
    }

    let mut kept = Kept {
        published: Published::default(),
        epoch: 0,
    };
    let newest = *bases.last().unwrap();
    let mut sealed = vec![];
    let mut tail = None;
    for (&base, (header, bytes)) in bases.iter().zip(headers) {
        if kept.published.watermark != base {
            return Err(OpenError::Gap { segment: base });
        }
        kept.epoch = kept.epoch.max(header.epoch);
        let path = segment_path(dir, base);
        let length = storage.length(&path, Purpose::Recovery)?;
        let scan = scan(storage, &log_id, base, &path, length, &mut kept)?;
        if base == newest {
            if matches!(scan.ending, Ending::Stopped)
                && closing::sealed_behind(storage, &path, &log_id, length, scan.end)?
            {
                return Err(OpenError::Corrupt {
                    segment: base,
                    position: scan.end,
                });
            }
            tail = Some((scan, bytes));
        } else if let Ending::Rolled(run) = scan.ending {
            sealed.push(run.summary);
            kept.published.roll();
        } else {
            // Every segment with a successor is sealed, and recovery never truncates one (I7).
            return Err(OpenError::Corrupt {
                segment: base,
                position: scan.end,
            });
        }
    }
    let (newest, newest_header) = tail.unwrap();
    Ok(Survey {
        log_id,
        published: kept.published,
        sealed,
        epoch: kept.epoch,
        newest_header,
        newest,
    })
}

fn header(
    storage: &ReadOnly<'_>,
    dir: &Path,
    base: u64,
) -> Result<(codec::Header, Vec<u8>), OpenError> {
    let bytes = storage.read(
        &segment_path(dir, base),
        Purpose::Open,
        0,
        codec::HEADER_LEN,
    )?;
    if bytes.len() == codec::HEADER_LEN
        && &bytes[..4] == b"SGLG"
        && bytes[6..8] == [64, 0]
        && bytes[40..60] == [0; 20]
        && codec::crc32c(&bytes[..60]) == u32::from_le_bytes(bytes[60..].try_into().unwrap())
    {
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if version != codec::FORMAT_VERSION {
            return Err(OpenError::UnsupportedVersion(version));
        }
    }
    match codec::decode_header(&bytes) {
        Ok(header) if header.base_offset == base => Ok((header, bytes)),
        _ => Err(OpenError::Corrupt {
            segment: base,
            position: 0,
        }),
    }
}

fn scan(
    storage: &ReadOnly<'_>,
    log: &codec::LogId,
    base: u64,
    path: &Path,
    length: u64,
    kept: &mut Kept,
) -> io::Result<Scan> {
    let mut position = codec::HEADER_LEN as u64;
    let mut state = RunState::Groups;
    let mut epoch = kept.epoch;
    let mut run = vec![];
    let mut last = None;
    let mut tails = BTreeMap::new();
    let mut keys = BTreeMap::new();
    let end = |at, ending, last, tails, keys| Scan {
        end: at,
        ending,
        last,
        tails,
        keys,
    };
    loop {
        if position == length && state == RunState::Groups {
            return Ok(end(position, Ending::Groups, last, tails, keys));
        }
        let Some(bytes) = storage.frame(path, Purpose::Recovery, position, length)? else {
            return Ok(end(
                state.stop(position),
                Ending::Stopped,
                last,
                tails,
                keys,
            ));
        };
        let frame = codec::decode(log, &bytes)
            .ok()
            .filter(|f| f.base_offset == kept.published.watermark && f.epoch >= epoch);
        let Some(frame) = frame else {
            return Ok(end(
                state.stop(position),
                Ending::Stopped,
                last,
                tails,
                keys,
            ));
        };
        let next = position + bytes.len() as u64;
        epoch = frame.epoch;
        match state.step(position, next, length, &frame.body) {
            Step::Group => {
                let group = frame.into_group().expect("a group step has a group body");
                kept.epoch = group.epoch;
                for (i, entry) in group.entries.iter().enumerate() {
                    if let Some(key) = entry.key {
                        let at = ActiveKey {
                            position: position as u32,
                            record: i as u32,
                        };
                        keys.insert(key.to_vec(), at);
                    }
                    tails.insert(
                        entry.stream.to_vec(),
                        Tail {
                            revision: entry.revision,
                            offset: group.base_offset + i as u64,
                            position,
                        },
                    );
                }
                kept.published
                    .publish(GroupSpan::of(position, bytes.len(), &group), &group);
                last = Some((position, bytes));
            }
            Step::Run(next) => {
                state = next;
                run.push(bytes);
            }
            Step::Stop { at } => return Ok(end(at, Ending::Stopped, last, tails, keys)),
            Step::Closed { start, footer } => {
                kept.epoch = epoch;
                run.push(bytes);
                let summary = SealedSummary {
                    base,
                    next: kept.published.watermark,
                    filter_at: start,
                    footer_at: footer,
                };
                let rolled = Ending::Rolled(Run {
                    summary,
                    frames: run,
                });
                return Ok(end(length, rolled, last, tails, keys));
            }
        }
        position = next;
    }
}
