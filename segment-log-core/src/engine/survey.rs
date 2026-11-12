use super::*;

pub(super) use read_only::ReadOnly;

mod read_only {
    use super::super::*;

    pub(in crate::engine) struct ReadOnly<'a>(&'a dyn Storage);

    impl<'a> ReadOnly<'a> {
        pub(in crate::engine) fn new(storage: &'a dyn Storage) -> Self {
            Self(storage)
        }

        pub(super) fn read(
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
    }
}

pub(super) struct Survey {
    pub log_id: codec::LogId,
    pub published: Published,
    pub keys: Keys,
    pub epoch: u64,
    pub newest_header: Vec<u8>,
    pub newest: Scan,
}

pub(super) struct Scan {
    /// The file position after the last valid frame.
    pub end: u64,
    pub reached_end_of_file: bool,
    /// Position and bytes of the last valid frame.
    pub last: Option<(u64, Vec<u8>)>,
}

/// What the scan has kept so far, across the segments it has read.
struct Kept {
    published: Published,
    keys: Keys,
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
    let (newest, newest_header) = tail.unwrap();
    Ok(Survey {
        log_id,
        published: kept.published,
        keys: kept.keys,
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

/// Reads the frames of one segment into `kept` and stops at the first frame that is not valid.
fn scan(
    storage: &ReadOnly<'_>,
    log: &codec::LogId,
    path: &Path,
    kept: &mut Kept,
) -> io::Result<Scan> {
    let mut position = codec::HEADER_LEN as u64;
    let mut last = None;
    loop {
        let envelope = storage.read(path, Purpose::Recovery, position, 8)?;
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
                Some(storage.read(path, Purpose::Recovery, position, length + 8)?)
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
