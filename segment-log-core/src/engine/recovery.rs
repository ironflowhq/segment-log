use super::*;
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

/// What the scan has kept so far, across the segments it has read.
struct Kept {
    published: Published,
    keys: Keys,
    /// The epoch that the next frame must not be lower than.
    epoch: u64,
}

struct Scan {
    /// The file position after the last valid frame.
    end: u64,
    reached_end_of_file: bool,
    /// Position and bytes of the last valid frame.
    last: Option<(u64, Vec<u8>)>,
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
    let mut bases: Vec<u64> = names
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

    if let Some(&newest) = bases.last() {
        let path = segment_path(dir, newest);
        let Value::Length(length) = call(storage, &path, Purpose::Open, Op::Open)? else {
            return Err(io::Error::other("storage did not return length").into());
        };
        let header = read_bytes(storage, &path, Purpose::Open, 0, codec::HEADER_LEN)?;
        let crc_valid = header.len() == codec::HEADER_LEN
            && codec::crc32c(&header[..60]) == u32::from_le_bytes(header[60..].try_into().unwrap());
        if length <= codec::HEADER_LEN as u64 && !crc_valid {
            call(storage, &path, Purpose::Open, Op::Remove)?;
            call(storage, dir, Purpose::Open, Op::SyncDir)?;
            bases.pop();
        }
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
        keys: Keys::new(log_id.clone()),
        epoch: 0,
    };
    let newest = *bases.last().unwrap();
    let mut tail = None;
    for (&base, (header, bytes)) in bases.iter().zip(headers) {
        if kept.published.watermark != base {
            return Err(OpenError::Gap { segment: base });
        }
        kept.epoch = kept.epoch.max(header.epoch);
        let scan = scan(storage, &log_id, &segment_path(dir, base), &mut kept)?;
        if base == newest {
            tail = Some((scan, bytes));
        } else if !scan.reached_end_of_file {
            // Every segment with a successor is sealed, and recovery never truncates one (I7).
            return Err(OpenError::Corrupt {
                segment: base,
                position: scan.end,
            });
        } else {
            kept.published.roll();
        }
    }
    let (scan, header) = tail.unwrap();
    let epoch = if created {
        1
    } else {
        kept.epoch.checked_add(1).ok_or(OpenError::Exhausted)?
    };

    let segment = segment_path(dir, newest);
    if !scan.reached_end_of_file {
        call(storage, &segment, Purpose::Recovery, Op::Truncate(scan.end))?;
    }
    let (position, bytes) = scan.last.unwrap_or((0, header));
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
    Ok(Recovered {
        active: Active {
            base: newest,
            length: scan.end,
        },
        log_id,
        lock,
        published: kept.published,
        epoch,
        keys: kept.keys,
    })
}

fn header(
    storage: &dyn Storage,
    dir: &Path,
    base: u64,
) -> Result<(codec::Header, Vec<u8>), OpenError> {
    let bytes = read_bytes(
        storage,
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

/// Reads the frames of one segment into `kept` and stops at the first frame that is not valid.
fn scan(
    storage: &dyn Storage,
    log: &codec::LogId,
    path: &Path,
    kept: &mut Kept,
) -> io::Result<Scan> {
    let mut position = codec::HEADER_LEN as u64;
    let mut last = None;
    loop {
        let envelope = read_bytes(storage, path, Purpose::Recovery, position, 8)?;
        if envelope.is_empty() {
            return Ok(Scan {
                end: position,
                reached_end_of_file: true,
                last,
            });
        }
        let frame = if envelope.len() == 8 {
            let length = u32::from_le_bytes(envelope[..4].try_into().unwrap()) as usize;
            if (codec::BODY_HEADER_LEN..=codec::GROUP_BODY_LIMIT).contains(&length) {
                Some(read_bytes(
                    storage,
                    path,
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
            .and_then(|b| codec::decode(log, b).ok())
            .and_then(codec::Frame::into_group)
            .filter(|g| g.base_offset == kept.published.watermark && g.epoch >= kept.epoch);
        let Some(group) = valid else {
            return Ok(Scan {
                end: position,
                reached_end_of_file: false,
                last,
            });
        };
        kept.epoch = group.epoch;
        let length = frame.as_ref().unwrap().len();
        kept.keys.recover(&group);
        kept.published
            .publish(GroupSpan::of(position, length, &group), &group);
        last = Some((position, frame.unwrap()));
        position += length as u64;
    }
}
