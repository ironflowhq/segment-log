use super::*;
use keys::{ActiveKey, KeyHash, SealedKeys};
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
    pub active: Active,
    pub active_keys: BTreeMap<Vec<u8>, ActiveKey>,
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
                log_id,
                base_offset: 0,
                epoch: 1,
            },
        )?;
        bases.push(0);
    }

    let mut found = survey::survey(&reader, dir, &bases, hash)?;
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
        Ending::Groups | Ending::Stopped => found.sealed.clone(),
    };
    // Step 11's reads come before any change, so that a bad key filter changes no file.
    let sealed_keys = SealedKeys::load_all(&reader, dir, &found.log_id, &summaries)?;

    let segment = segment_path(dir, newest);
    if let Ending::Stopped = found.newest.ending {
        call(
            storage,
            &segment,
            Purpose::Recovery,
            Op::Truncate(found.newest.end),
        )?;
    }
    let (position, bytes) = match &found.newest.ending {
        Ending::Rolled { run, .. } | Ending::Sealed(run) => (run.start, run.frames.concat()),
        Ending::Groups | Ending::Stopped => found.newest.last.unwrap_or((0, found.newest_header)),
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
        (Ending::Rolled { .. }, Some(epoch)) => {
            let next = found.published.watermark;
            create_segment(
                storage,
                dir,
                Purpose::Recovery,
                &codec::Header {
                    log_id: *found.log_id.bytes(),
                    base_offset: next,
                    epoch,
                },
            )?;
            found.published.roll(found.epoch, epoch);
            (epoch, Active::new(next), BTreeMap::new())
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
            active,
            active_keys,
            sealed_keys,
        }),
    })
}
