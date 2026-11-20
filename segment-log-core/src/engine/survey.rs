use super::*;
use closing::{RunState, Step, Tail};
use keys::{ActiveKey, KeyHash, KeyMap, SealedSummary};

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

        pub(in crate::engine) fn length(&self, path: &Path, purpose: Purpose) -> io::Result<u64> {
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
    pub hash: KeyHash,
    pub published: Published,
    pub sealed: Vec<SealedSummary>,
    pub epoch: u64,
    /// The append time of the newest group that open keeps, or 0 in a log with no group. The
    /// writer never stamps a lower one (ADR 0064, rule 2).
    pub append_time: u64,
    /// Its format version may be older than the build's.
    pub newest_header: codec::Header,
    pub newest: Scan,
    /// The second-newest segment's closing run, rebuilt because the writer had not written it
    /// (ADR 0061, rule 8). Its summary is the last in `sealed`, and its key filter is not on disk.
    pub pending: Option<Rebuilt>,
}

pub(super) struct Rebuilt {
    pub start: u64,
    pub length: u64,
    pub run: [Vec<u8>; 4],
    pub summary: SealedSummary,
}

pub(super) struct Scan {
    pub end: u64,
    pub ending: Ending,
    pub last: Option<(u64, Vec<u8>)>,
    /// Whether the scan's last frame is a close frame (ADR 0071).
    pub closed: bool,
    pub tails: BTreeMap<Vec<u8>, Tail>,
    pub keys: KeyMap,
}

pub(super) enum Ending {
    Groups,
    Stopped,
    Rolled { run: Run, summary: SealedSummary },
    Sealed(Run),
}

pub(super) struct Run {
    pub start: u64,
    pub frames: Vec<Vec<u8>>,
}

/// What the scan has kept so far, across the segments it has read.
pub(super) struct Kept {
    pub published: Published,
    /// The epoch that the next frame must not be lower than.
    pub epoch: u64,
    /// The segment base and position of the newest group kept so far.
    pub last_group: Option<(u64, u64)>,
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
    hash: impl FnOnce(codec::LogId) -> KeyHash,
    version: u16,
) -> Result<Survey, OpenError> {
    let headers = bases
        .iter()
        .map(|&base| header(storage, dir, base, version))
        .collect::<Result<Vec<_>, _>>()?;
    let log_id = codec::LogId::new(headers[0].log_id);
    for (&base, header) in bases.iter().zip(&headers) {
        if header.log_id != *log_id.bytes() {
            return Err(OpenError::LogIdMismatch { segment: base });
        }
    }
    // A writer never goes back a version (ADR 0064, rule 12), so this comes only from a hand
    // copy or damage, and the older header is the bad one (ADR 0066).
    for (pair, &base) in headers.windows(2).zip(&bases[1..]) {
        if pair[1].version < pair[0].version {
            return Err(OpenError::Corrupt {
                segment: base,
                position: 0,
            });
        }
    }
    let hash = hash(log_id.clone());

    let mut kept = Kept {
        published: Published::default(),
        epoch: 0,
        last_group: None,
    };
    let mut sealed = vec![];
    let mut pending = None;
    let newest_base = *bases.last().unwrap();
    let newest_epoch = headers.last().unwrap().epoch;
    let mut headers = headers.into_iter();
    for (pair, header) in bases.windows(2).zip(headers.by_ref()) {
        let [base, successor] = *pair else {
            unreachable!()
        };
        if kept.published.watermark != base {
            return Err(OpenError::Gap { segment: base });
        }
        kept.epoch = kept.epoch.max(header.epoch);
        let summary = match summarize(storage, &log_id, dir, base, successor, &mut kept) {
            Err(error @ OpenError::Corrupt { .. }) if successor == newest_base => {
                // The writer that created the newest segment also built this run.
                let at = (base, successor, newest_epoch, closes(&header));
                let rebuilt = rebuild(storage, &log_id, &hash, dir, at, &mut kept)?.ok_or(error)?;
                let summary = rebuilt.summary;
                pending = Some(rebuilt);
                summary
            }
            summary => summary?,
        };
        sealed.push(summary);
    }
    let header = headers.next().unwrap();
    let base = *bases.last().unwrap();
    if kept.published.watermark != base {
        return Err(OpenError::Gap { segment: base });
    }
    kept.epoch = kept.epoch.max(header.epoch);
    kept.published.newest.epoch_floor = kept.epoch;
    let path = segment_path(dir, base);
    let length = storage.length(&path, Purpose::Recovery)?;
    let at = (path.as_path(), closes(&header));
    let newest = scan(storage, &log_id, &hash, base, at, length, &mut kept)?;
    if matches!(newest.ending, Ending::Stopped)
        && (closing::sealed_behind(storage, &path, &log_id, length, newest.end)?
            || !proofs(
                storage,
                at,
                &log_id,
                (length, newest.end, kept.published.watermark),
                1,
            )?
            .is_empty())
    {
        return Err(OpenError::Corrupt {
            segment: base,
            position: newest.end,
        });
    }
    // A roll or seal first writes the pending run, so a closed newest segment rules it out.
    if let Some(rebuilt) = &pending
        && matches!(newest.ending, Ending::Rolled { .. } | Ending::Sealed(_))
    {
        return Err(OpenError::Corrupt {
            segment: rebuilt.summary.base,
            position: rebuilt.start,
        });
    }
    let append_time = match (&newest.last, kept.last_group) {
        (Some((_, bytes)), _) => group_time(&log_id, bytes).expect("scan keeps valid groups"),
        // A newest segment with no group: the newest group ends the segment before it.
        (None, Some((segment, position))) => {
            let path = segment_path(dir, segment);
            let length = storage.length(&path, Purpose::Recovery)?;
            storage
                .frame(&path, Purpose::Recovery, position, length)?
                .and_then(|bytes| group_time(&log_id, &bytes))
                .ok_or(OpenError::Corrupt { segment, position })?
        }
        (None, None) => 0,
    };
    Ok(Survey {
        log_id,
        hash,
        published: kept.published,
        sealed,
        epoch: kept.epoch,
        append_time,
        newest_header: header,
        newest,
        pending,
    })
}

/// Whether a segment's format version lets it hold close frames (ADR 0071).
pub(in crate::engine) fn closes(header: &codec::Header) -> bool {
    header.version >= codec::CLOSE_VERSION
}

/// A frame after a scan's stop that proves the bytes at the stop were durable: a group frame
/// (ADR 0068) or a close frame (ADR 0071).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::engine) struct Proof {
    pub position: u64,
    /// The first offset after the frame's records.
    pub end: u64,
    /// The bytes of a close frame, which a repair may copy past its seal run (ADR 0071); `None`
    /// for a group frame.
    pub close: Option<Vec<u8>>,
}

/// Every proof after `at`, in file order. Frames carry no marker, so each byte position is a
/// candidate. Only one group frame of a segment can be unsynced, its last, so a group frame of
/// this log whose `base_offset` is at least `next` was written after a durable one. A clean drop
/// writes a close frame only after its last group was synced, so one whose `base_offset` is past
/// `next` proves the same; one at `next` or below is from an earlier session. A lookalike copied
/// into a torn payload has a lower `base_offset`, and closing frames never count. Stops after
/// `limit` proofs: open needs one, and the repair plan needs them all.
pub(in crate::engine) fn proofs(
    storage: &ReadOnly<'_>,
    (path, closes): (&Path, bool),
    log: &codec::LogId,
    (length, at, next): (u64, u64, u64),
    limit: usize,
) -> io::Result<Vec<Proof>> {
    const MAX_FRAME: usize = codec::ENVELOPE_LEN + codec::FRAME_BODY_LIMIT;
    // Two frames' worth, so each refill moves at least one frame on, and memory stays bounded.
    const WINDOW: u64 = 2 * MAX_FRAME as u64;
    let mut found = vec![];
    let (mut base, mut window) = (at, vec![]);
    let mut position = at + 1;
    while found.len() < limit && position + codec::ENVELOPE_LEN as u64 <= length {
        let window_end = base + window.len() as u64;
        if position + MAX_FRAME as u64 > window_end && window_end < length {
            base = position;
            let len = WINDOW.min(length - base) as usize;
            window = storage.read(path, Purpose::Recovery, base, len)?;
            if window.len() != len {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
        let i = (position - base) as usize;
        let body = u32::from_le_bytes(window[i..i + 4].try_into().unwrap()) as usize;
        let frame_end = i + codec::ENVELOPE_LEN + body;
        // Cheap checks first, so that a CRC runs only for a plausible group frame.
        let frame = &window[i..frame_end.min(window.len())];
        let proof = if (codec::BODY_HEADER_LEN..=codec::FRAME_BODY_LIMIT).contains(&body)
            && frame_end <= window.len()
            && codec::proof_candidate(frame, next)
        {
            match codec::decode(log, frame) {
                Ok(codec::Frame {
                    base_offset,
                    body: codec::Body::Group { entries, .. },
                    ..
                }) => Some((base_offset + entries.len() as u64, None)),
                Ok(codec::Frame {
                    base_offset,
                    body: codec::Body::Close,
                    ..
                }) if closes => Some((base_offset, Some(frame.to_vec()))),
                _ => None,
            }
        } else {
            None
        };
        match proof {
            Some((end, close)) => {
                found.push(Proof {
                    position,
                    end,
                    close,
                });
                position = base + frame_end as u64;
            }
            None => position += 1,
        }
    }
    Ok(found)
}

fn group_time(log: &codec::LogId, frame: &[u8]) -> Option<u64> {
    codec::decode(log, frame)
        .ok()?
        .into_group()
        .map(|g| g.append_time)
}

/// Rule 8 of ADR 0061: the second-newest segment may end in its groups, or in a closing run that
/// a crash tore, because the writer writes a roll's run after the next segment exists. Scans it,
/// as step 6 scans the newest, and rebuilds the run that the writer would have written. `None`
/// means that the segment is not that case, and the caller keeps its error.
fn rebuild(
    storage: &ReadOnly<'_>,
    log: &codec::LogId,
    hash: &KeyHash,
    dir: &Path,
    (base, successor, epoch, closes): (u64, u64, u64, bool),
    kept: &mut Kept,
) -> Result<Option<Rebuilt>, OpenError> {
    let path = segment_path(dir, base);
    let length = storage.length(&path, Purpose::Recovery)?;
    kept.published.newest.epoch_floor = kept.epoch;
    let scan = scan(storage, log, hash, base, (&path, closes), length, kept)?;
    match scan.ending {
        Ending::Groups => (),
        Ending::Stopped if !closing::sealed_behind(storage, &path, log, length, scan.end)? => (),
        _ => return Ok(None),
    }
    if matches!(scan.ending, Ending::Stopped)
        && !proofs(
            storage,
            (&path, closes),
            log,
            (length, scan.end, kept.published.watermark),
            1,
        )?
        .is_empty()
    {
        return Err(OpenError::Corrupt {
            segment: base,
            position: scan.end,
        });
    }
    // A scan that stopped before the successor's first offset lost acknowledged records.
    if kept.published.watermark != successor {
        return Err(OpenError::Gap { segment: successor });
    }
    if kept.epoch > epoch {
        return Ok(None);
    }
    let mut entries = hash.entries(&scan.keys);
    let run = closing::roll_run(
        log,
        base,
        epoch,
        successor,
        scan.end,
        &scan.tails,
        &mut entries,
    );
    let summary = SealedSummary {
        base,
        next: successor,
        filter_at: scan.end,
        footer_at: scan.end + (run[0].len() + run[1].len()) as u64,
    };
    kept.published.roll(epoch, epoch);
    kept.epoch = epoch;
    Ok(Some(Rebuilt {
        start: scan.end,
        length,
        run,
        summary,
    }))
}

/// Step 5 for a segment with a successor, which proves that its locator is its last frame. Reads
/// only the locator and the footer it names, so that open time does not grow with the history
/// (ORACLE-13). Recovery never truncates a sealed segment (I7), so a bad byte is `Corrupt`.
fn summarize(
    storage: &ReadOnly<'_>,
    log: &codec::LogId,
    dir: &Path,
    base: u64,
    successor: u64,
    kept: &mut Kept,
) -> Result<SealedSummary, OpenError> {
    let corrupt = |position| OpenError::Corrupt {
        segment: base,
        position,
    };
    let path = segment_path(dir, base);
    let length = storage.length(&path, Purpose::Summary)?;
    let header_end = codec::HEADER_LEN as u64;
    let locator_at = length
        .saturating_sub(codec::LOCATOR_LEN as u64)
        .max(header_end);
    if locator_at + codec::LOCATOR_LEN as u64 != length {
        return Err(corrupt(locator_at));
    }
    let bytes = storage.read(&path, Purpose::Summary, locator_at, codec::LOCATOR_LEN)?;
    let Ok(codec::Frame {
        epoch,
        base_offset: next,
        body: codec::Body::Locator {
            footer_position: footer_at,
        },
    }) = codec::decode(log, &bytes)
    else {
        return Err(corrupt(locator_at));
    };
    let footer_len = locator_at
        .checked_sub(footer_at)
        .filter(|len| {
            footer_at >= header_end
                && (8 + codec::FOOTER_HEADER_LEN as u64..=8 + codec::FRAME_BODY_LIMIT as u64)
                    .contains(len)
        })
        .ok_or(corrupt(locator_at))?;
    let bytes = storage.read(&path, Purpose::Summary, footer_at, footer_len as usize)?;
    let Ok(codec::Frame {
        epoch: footer_epoch,
        base_offset: footer_next,
        body: codec::Body::Footer(footer),
    }) = codec::decode(log, &bytes)
    else {
        return Err(corrupt(footer_at));
    };
    let filter_at = footer
        .key_filter_position
        .map(u64::from)
        .filter(|at| (header_end + 1..footer_at).contains(at))
        .ok_or(corrupt(footer_at))?;
    if footer_next != successor || next != successor {
        return Err(OpenError::Gap { segment: successor });
    }
    if footer_epoch < kept.epoch {
        return Err(corrupt(footer_at));
    }
    if epoch < footer_epoch {
        return Err(corrupt(locator_at));
    }
    if footer.streams.iter().any(|tail| {
        !(base..next).contains(&tail.last_offset)
            || !(header_end..filter_at).contains(&tail.last_position)
    }) {
        return Err(corrupt(footer_at));
    }
    let summary = SealedSummary {
        base,
        next,
        filter_at,
        footer_at,
    };
    kept.published
        .opened(&summary, &footer, kept.epoch..=footer_epoch);
    // A segment's newest group holds some stream's last record in it. A footer with no stream
    // names no group, so an earlier segment's newest group stays the newest.
    kept.last_group = footer
        .streams
        .iter()
        .map(|tail| (base, tail.last_position))
        .max_by_key(|&(_, position)| position)
        .or(kept.last_group);
    // The locator is the segment's last frame, and epochs never decrease within a segment.
    kept.epoch = epoch;
    Ok(summary)
}

/// Each segment is checked against its own format version, from the oldest a build reads up to
/// `version`, the build's own (ADR 0064, rules 11 and 12).
pub(super) fn header(
    storage: &ReadOnly<'_>,
    dir: &Path,
    base: u64,
    version: u16,
) -> Result<codec::Header, OpenError> {
    let bytes = storage.read(
        &segment_path(dir, base),
        Purpose::Open,
        0,
        codec::HEADER_LEN,
    )?;
    match codec::decode_header(&bytes) {
        Ok(header) if !(codec::MIN_FORMAT_VERSION..=version).contains(&header.version) => {
            Err(OpenError::UnsupportedVersion(header.version))
        }
        Ok(header) if header.base_offset == base => Ok(header),
        _ => Err(OpenError::Corrupt {
            segment: base,
            position: 0,
        }),
    }
}

pub(super) fn scan(
    storage: &ReadOnly<'_>,
    log: &codec::LogId,
    hash: &KeyHash,
    base: u64,
    (path, closes): (&Path, bool),
    length: u64,
    kept: &mut Kept,
) -> Result<Scan, OpenError> {
    let mut position = codec::HEADER_LEN as u64;
    let mut state = RunState::Groups;
    let mut epoch = kept.epoch;
    let mut run = vec![];
    let mut last = None;
    let mut closed = false;
    let mut tails = BTreeMap::new();
    let mut keys = KeyMap::new();
    let end = |at, ending, last, closed, tails, keys| Scan {
        end: at,
        ending,
        last,
        closed,
        tails,
        keys,
    };
    loop {
        if position == length && state == RunState::Groups {
            return Ok(end(position, Ending::Groups, last, closed, tails, keys));
        }
        let Some(bytes) = storage.frame(path, Purpose::Recovery, position, length)? else {
            return Ok(end(
                state.stop(position),
                Ending::Stopped,
                last,
                closed,
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
                closed,
                tails,
                keys,
            ));
        };
        let next = position + bytes.len() as u64;
        epoch = frame.epoch;
        // A close frame holds no record; a version 3 segment never holds one (ADR 0071).
        if closes && state == RunState::Groups && matches!(frame.body, codec::Body::Close) {
            kept.epoch = epoch;
            closed = true;
            position = next;
            continue;
        }
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
                    let tail = Tail {
                        revision: entry.revision,
                        offset: group.base_offset + i as u64,
                        position,
                    };
                    match tails.get_mut(entry.stream) {
                        Some(known) => *known = tail,
                        None => {
                            tails.insert(entry.stream.to_vec(), tail);
                        }
                    }
                }
                closed = false;
                kept.published
                    .publish(GroupSpan::of(position, bytes.len(), &group), &group);
                kept.last_group = Some((base, position));
                last = Some((position, bytes));
            }
            Step::Run(next) => {
                state = next;
                run.push(bytes);
            }
            Step::Stop { at } => {
                return Ok(end(at, Ending::Stopped, last, closed, tails, keys));
            }
            Step::Rolled { start, footer } => {
                kept.epoch = epoch;
                run.push(bytes);
                let next = kept.published.watermark;
                let mut entries = hash.entries(&keys);
                let built = closing::roll_run(log, base, epoch, next, start, &tails, &mut entries);
                matches_groups(&run, &built, base, start)?;
                let summary = SealedSummary {
                    base,
                    next: kept.published.watermark,
                    filter_at: start,
                    footer_at: footer,
                };
                let run = Run { start, frames: run };
                let rolled = Ending::Rolled { run, summary };
                return Ok(end(length, rolled, last, closed, tails, keys));
            }
            Step::Sealed { start } => {
                kept.epoch = epoch;
                run.push(bytes);
                let next = kept.published.watermark;
                let built = closing::seal_run(log, epoch, next, start, &tails);
                matches_groups(&run, &built, base, start)?;
                let sealed = Ending::Sealed(Run { start, frames: run });
                return Ok(end(length, sealed, last, closed, tails, keys));
            }
        }
        position = next;
    }
}

/// A closing run passes every CRC only if a writer or the device made it, so one that differs from
/// the run its groups produce is a bug or damage the CRC missed, never a torn write (ADR 0054).
fn matches_groups(
    run: &[Vec<u8>],
    built: &[Vec<u8>],
    segment: u64,
    start: u64,
) -> Result<(), OpenError> {
    if run.iter().flatten().eq(built.iter().flatten()) {
        Ok(())
    } else {
        Err(OpenError::Corrupt {
            segment,
            position: start,
        })
    }
}
