use super::*;
pub(super) struct Recovered {
    pub segment: PathBuf,
    pub lock: Box<dyn Lock>,
    pub published: Published,
    pub epoch: u64,
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
    let Value::Names(names) = call(storage, dir, Purpose::Open, Op::List)? else {
        return Err(io::Error::other("storage did not list directory").into());
    };
    let names: Vec<_> = names
        .into_iter()
        .filter(|n| {
            n.to_str().is_some_and(|s| {
                s.len() == 24 && s.ends_with(".seg") && s[..20].bytes().all(|b| b.is_ascii_digit())
            })
        })
        .collect();
    if names.len() > 1 {
        return Err(io::Error::from(io::ErrorKind::Unsupported).into());
    }
    let segment = dir.join(
        names
            .first()
            .cloned()
            .unwrap_or_else(|| "00000000000000000000.seg".into()),
    );
    let mut created = names.is_empty();
    if !created {
        let Value::Length(len) = call(storage, &segment, Purpose::Open, Op::Open)? else {
            return Err(io::Error::other("storage did not return length").into());
        };
        let header = read_bytes(storage, &segment, Purpose::Open, 0, 64)?;
        let crc_valid = header.len() == 64
            && codec::crc32c(&header[..60]) == u32::from_le_bytes(header[60..].try_into().unwrap());
        if len <= 64 && !crc_valid {
            call(storage, &segment, Purpose::Open, Op::Remove)?;
            call(storage, dir, Purpose::Open, Op::SyncDir)?;
            created = true;
        }
    }
    let segment = if created {
        dir.join("00000000000000000000.seg")
    } else {
        segment
    };
    if created {
        let Value::Bytes(id) = call(storage, dir, Purpose::Open, Op::Random(16))? else {
            return Err(io::Error::other("storage did not return random bytes").into());
        };
        let log_id = id
            .try_into()
            .map_err(|_| io::Error::other("invalid random byte count"))?;
        let header = codec::encode_header(&codec::Header {
            log_id,
            base_offset: 0,
            epoch: 1,
        });
        call(storage, &segment, Purpose::Open, Op::Create)?;
        call(
            storage,
            &segment,
            Purpose::Open,
            Op::Write(0, header.to_vec()),
        )?;
        call(storage, &segment, Purpose::Open, Op::SyncData)?;
        call(storage, dir, Purpose::Open, Op::SyncDir)?;
    }
    let bytes = read_bytes(storage, &segment, Purpose::Open, 0, 64)?;
    if bytes.len() == 64
        && &bytes[..4] == b"SGLG"
        && bytes[6..8] == [64, 0]
        && bytes[40..60] == [0; 20]
        && codec::crc32c(&bytes[..60]) == u32::from_le_bytes(bytes[60..].try_into().unwrap())
    {
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if version != 0 {
            return Err(OpenError::UnsupportedVersion(version));
        }
    }
    let header = codec::decode_header(&bytes).map_err(|_| OpenError::Corrupt {
        segment: 0,
        position: 0,
    })?;
    if segment.file_name().unwrap().to_str().unwrap() != format!("{:020}.seg", header.base_offset) {
        return Err(OpenError::Corrupt {
            segment: header.base_offset,
            position: 0,
        });
    }
    if header.base_offset != 0 {
        return Err(OpenError::Gap {
            segment: header.base_offset,
        });
    }
    let mut published = Published::default();
    let mut epoch = header.epoch;
    let mut position = 64;
    let mut tail = (0, bytes);
    let mut truncate = false;
    loop {
        let envelope = read_bytes(storage, &segment, Purpose::Recovery, position, 8)?;
        if envelope.is_empty() {
            break;
        }
        let frame = if envelope.len() == 8 {
            let length = u32::from_le_bytes(envelope[..4].try_into().unwrap()) as usize;
            if (24..=8 * 1024 * 1024).contains(&length) {
                Some(read_bytes(
                    storage,
                    &segment,
                    Purpose::Recovery,
                    position,
                    length + 8,
                )?)
            } else {
                None
            }
        } else {
            None
        };
        let valid = frame
            .as_deref()
            .and_then(|b| codec::decode(b).ok())
            .filter(|g| g.base_offset == published.watermark && g.epoch >= epoch);
        let Some(group) = valid else {
            truncate = true;
            break;
        };
        epoch = group.epoch;
        let bytes = frame.as_ref().unwrap();
        publish(
            &mut published,
            Frame {
                position,
                length: bytes.len(),
                base: group.base_offset,
                end: group.base_offset + group.entries.len() as u64,
            },
            &group,
        );
        tail = (position, frame.unwrap());
        position += tail.1.len() as u64;
    }
    let epoch = if created {
        1
    } else {
        epoch.checked_add(1).ok_or(OpenError::Exhausted)?
    };
    if truncate {
        call(storage, &segment, Purpose::Recovery, Op::Truncate(position))?;
    }
    call(
        storage,
        &segment,
        Purpose::Recovery,
        Op::Write(tail.0, tail.1),
    )?;
    call(storage, &segment, Purpose::Recovery, Op::SyncAll)?;
    call(storage, dir, Purpose::Recovery, Op::SyncDir)?;
    let parent = dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    call(storage, parent, Purpose::Recovery, Op::SyncDir)?;
    Ok(Recovered {
        segment,
        lock,
        published,
        epoch,
    })
}
