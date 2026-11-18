use super::*;
use keys::{KeyHash, KeyMap, SealedKeys};
use survey::{Ending, ReadOnly};
pub(super) struct Recovered {
    pub log_id: codec::LogId,
    pub hash: KeyHash,
    pub published: Published,
    pub state: State,
}

pub(super) enum State {
    Sealed,
    Writable(Writable),
}

pub(super) struct Writable {
    pub lock: Box<dyn Lock>,
    pub epoch: u64,
    pub append_time: u64,
    pub active: Active,
    pub active_keys: KeyMap,
    pub sealed_keys: Vec<SealedKeys>,
}
impl From<io::Error> for OpenError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(super) fn open(
    storage: &dyn Storage,
    dir: &Path,
    hash: impl FnOnce(codec::LogId) -> KeyHash,
    version: u16,
) -> Result<Recovered, OpenError> {
    match call(storage, dir, Purpose::Open, Op::CreateDir) {
        Ok(_) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    let lock_path = dir.join("LOCK");
    match call(storage, &lock_path, Purpose::Open, Op::Create) {
        Ok(_) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    let lock = storage.lock(&lock_path).map_err(|e| {
        if e.kind() == io::ErrorKind::WouldBlock {
            OpenError::Locked
        } else {
            OpenError::Io(e)
        }
    })?;
    let reader = ReadOnly::new(storage);
    let mut bases = survey::segment_bases(&reader, dir)?;
    if let Some(&newest) = bases.last()
        && survey::is_torn_header(&reader, dir, newest)?
    {
        call(
            storage,
            &segment_path(dir, newest),
            Purpose::Open,
            Op::Remove,
        )?;
        call(storage, dir, Purpose::Open, Op::SyncDir)?;
        bases.pop();
    }
    let created = bases.is_empty();
    if created {
        let Value::Bytes(id) = call(storage, dir, Purpose::Open, Op::Random(16))? else {
            return Err(io::Error::other("storage did not return random bytes").into());
        };
        let log_id = id
            .try_into()
            .map_err(|_| io::Error::other("invalid random byte count"))?;
        create_segment(
            storage,
            dir,
            Purpose::Open,
            &codec::Header {
                version,
                log_id,
                base_offset: 0,
                epoch: 1,
            },
        )?;
        bases.push(0);
    }

    let mut found = survey::survey(&reader, dir, &bases, hash, version)?;
    let newest = *bases.last().unwrap();
    let epoch = match found.newest.ending {
        // A sealed log starts no writer, so it needs no next epoch.
        Ending::Sealed(_) => None,
        _ if created => Some(1),
        _ => Some(found.epoch.checked_add(1).ok_or(OpenError::Exhausted)?),
    };

    let summaries = match &found.newest.ending {
        // A sealed log never checks a key again.
        Ending::Sealed(_) => vec![],
        Ending::Rolled { summary, .. } => [found.sealed.as_slice(), &[*summary]].concat(),
        // A rebuilt run's key filter is not on disk yet.
        Ending::Groups | Ending::Stopped if found.pending.is_some() => {
            found.sealed[..found.sealed.len() - 1].to_vec()
        }
        Ending::Groups | Ending::Stopped => found.sealed.clone(),
    };
    // Step 11's reads come before any change, so that a bad key filter changes no file.
    let mut sealed_keys = SealedKeys::load_all(&reader, dir, &found.log_id, &summaries)?;

    // The rebuilt closing run is the first change, so the newest segment's own recovery below
    // happens with every older segment sealed (ADR 0061, rule 8).
    if let Some(rebuilt) = found.pending.take() {
        let path = segment_path(dir, rebuilt.summary.base);
        if rebuilt.length > rebuilt.start {
            call(
                storage,
                &path,
                Purpose::Recovery,
                Op::Truncate(rebuilt.start),
            )?;
        }
        write_run(storage, &path, &rebuilt.run, rebuilt.start)?;
        let filter = &rebuilt.run[0];
        sealed_keys.push(SealedKeys::written(rebuilt.summary, &found.log_id, filter));
    }

    let segment = segment_path(dir, newest);
    if let Ending::Stopped = found.newest.ending {
        call(
            storage,
            &segment,
            Purpose::Recovery,
            Op::Truncate(found.newest.end),
        )?;
    }
    // A writer never appends to a segment of an older format version (ADR 0064, rule 12). An
    // older active segment that holds only its header gets the build's header in its place, in
    // this same one-sector rewrite, because a roll would reuse its file name. One with groups is
    // closed in its own version and rolled below.
    let older = found.newest_header.version < version;
    let (position, bytes) = match &found.newest.ending {
        Ending::Rolled { run, .. } | Ending::Sealed(run) => (run.start, run.frames.concat()),
        Ending::Groups | Ending::Stopped => found.newest.last.take().unwrap_or_else(|| {
            // A valid header encodes back to its own bytes, so an unchanged version rewrites the
            // same bytes, as ADR 0015 needs.
            let header = codec::Header {
                version: version.max(found.newest_header.version),
                ..found.newest_header
            };
            (0, codec::encode_header(&header).to_vec())
        }),
    };
    let close = older && position > 0 && !matches!(found.newest.ending, Ending::Sealed(_));
    // The V2 control: different bytes over one acknowledged sector in place of the identical
    // rewrite. The byte sits past the frame's envelope, so the frame's CRC fails at the next open.
    #[cfg(feature = "rewrite-control")]
    let bytes = {
        let mut bytes = bytes;
        if let Some(byte) = bytes.get_mut(32) {
            *byte ^= 0xff;
        }
        bytes
    };
    call(
        storage,
        &segment,
        Purpose::Recovery,
        Op::Write(position, bytes),
    )?;
    call(storage, &segment, Purpose::Recovery, Op::SyncAll)?;
    call(storage, dir, Purpose::Recovery, Op::SyncDir)?;
    let parent = dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    call(storage, parent, Purpose::Recovery, Op::SyncDir)?;
    let (epoch, active, active_keys) = match (found.newest.ending, epoch) {
        (Ending::Sealed(_), _) | (_, None) => {
            drop(lock);
            return Ok(Recovered {
                log_id: found.log_id,
                hash: found.hash,
                published: found.published,
                state: State::Sealed,
            });
        }
        (Ending::Groups | Ending::Stopped, Some(epoch)) if close => {
            // The older segment's closing run, in its own version, under the new writer epoch.
            // Written in two steps, as a writer writes it, so that a crash leaves a torn run that
            // the next open truncates and writes again, or a whole one that it rolls past.
            let (start, next) = (found.newest.end, found.published.watermark);
            let mut entries = found.hash.entries(&found.newest.keys);
            let run = closing::roll_run(
                &found.log_id,
                newest,
                epoch,
                next,
                start,
                &found.newest.tails,
                &mut entries,
            );
            let footer_at = write_run(storage, &segment, &run, start)?;
            let summary = keys::SealedSummary {
                base: newest,
                next,
                filter_at: start,
                footer_at,
            };
            sealed_keys.push(SealedKeys::written(summary, &found.log_id, &run[0]));
            roll_into(storage, dir, version, &found.log_id, next, epoch)?;
            found.published.roll(epoch, epoch);
            (epoch, Active::new(next), KeyMap::new())
        }
        (Ending::Rolled { .. }, Some(epoch)) => {
            let next = found.published.watermark;
            roll_into(storage, dir, version, &found.log_id, next, epoch)?;
            found.published.roll(found.epoch, epoch);
            (epoch, Active::new(next), KeyMap::new())
        }
        (Ending::Groups | Ending::Stopped, Some(epoch)) => {
            let active = Active::recovered(newest, found.newest.end, found.newest.tails);
            (epoch, active, found.newest.keys)
        }
    };
    Ok(Recovered {
        log_id: found.log_id,
        hash: found.hash,
        published: found.published,
        state: State::Writable(Writable {
            lock,
            epoch,
            append_time: found.append_time,
            active,
            active_keys,
            sealed_keys,
        }),
    })
}

/// Starts the next segment, in the build's own format version.
fn roll_into(
    storage: &dyn Storage,
    dir: &Path,
    version: u16,
    log_id: &codec::LogId,
    next: u64,
    epoch: u64,
) -> io::Result<()> {
    let header = codec::Header {
        version,
        log_id: *log_id.bytes(),
        base_offset: next,
        epoch,
    };
    create_segment(storage, dir, Purpose::Recovery, &header)
}

/// Writes a roll's run at `start` in two steps, as the writer writes it, so that a crash leaves a
/// run that a later open either proves from its locator and footer, or truncates or rebuilds.
/// Returns the footer's position.
fn write_run(
    storage: &dyn Storage,
    path: &Path,
    run: &[Vec<u8>; 4],
    start: u64,
) -> io::Result<u64> {
    let [filter, index, footer, locator] = run;
    let footer_at = start + (filter.len() + index.len()) as u64;
    for (at, bytes) in [(start, [filter, index]), (footer_at, [footer, locator])] {
        let bytes = bytes.map(Vec::as_slice).concat();
        call(storage, path, Purpose::Recovery, Op::Write(at, bytes))?;
        call(storage, path, Purpose::Recovery, Op::SyncAll)?;
    }
    Ok(footer_at)
}
