use super::*;
use survey::{Ending, ReadOnly};
pub(super) struct Recovered {
    pub active: Active,
    pub log_id: codec::LogId,
    pub lock: Box<dyn Lock>,
    pub published: Published,
    pub epoch: u64,
    pub keys: Keys,
}
impl From<io::Error> for OpenError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(super) fn open(storage: &dyn Storage, dir: &Path) -> Result<Recovered, OpenError> {
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

    let mut found = survey::survey(&reader, dir, &bases)?;
    let newest = *bases.last().unwrap();
    let epoch = if created {
        1
    } else {
        found.epoch.checked_add(1).ok_or(OpenError::Exhausted)?
    };

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
        Ending::Rolled(run) => (run.start, run.frames.concat()),
        _ => found.newest.last.unwrap_or((0, found.newest_header)),
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
    let active = if let Ending::Rolled(_) = found.newest.ending {
        // A crash came after the roll's flush and before the next segment existed.
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
        found.published.roll();
        Active::new(next)
    } else {
        Active {
            base: newest,
            length: found.newest.end,
            tails: found.newest.tails,
        }
    };
    Ok(Recovered {
        active,
        log_id: found.log_id,
        lock,
        published: found.published,
        epoch,
        keys: found.keys,
    })
}
