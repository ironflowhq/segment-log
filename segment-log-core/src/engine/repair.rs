//! The operator repair tool's core (ADR 0065). Hidden from the docs: the command line in api.md
//! carries the promise, not this module.
use super::*;
use survey::{Ending, Kept, ReadOnly};

/// A finding's class, as ADR 0065's table names it. `TornTail` is a state that open recovers on
/// its own, so it is not damage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    A,
    B,
    C,
    D,
    E,
    TornTail,
}

impl Class {
    /// The name in the JSON output of api.md.
    pub fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
            Self::TornTail => "torn-tail",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub class: Class,
    pub segment: LogOffset,
    pub position: u64,
    pub message: String,
}

/// A frame near a finding, as its envelope and body header read. `valid` is whether the frame
/// decodes on its own bytes; the scan's offset and epoch continuity are not part of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    pub segment: LogOffset,
    pub position: u64,
    pub kind: &'static str,
    pub epoch: u64,
    pub base_offset: u64,
    pub count: u32,
    pub valid: bool,
}

#[derive(Debug, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub frames: Vec<FrameInfo>,
}

impl Report {
    /// Whether any finding is damage, rather than a state that open recovers on its own.
    pub fn damaged(&self) -> bool {
        self.findings.iter().any(|f| f.class != Class::TornTail)
    }

    fn note(&mut self, class: Class, segment: u64, position: u64, message: impl Into<String>) {
        self.findings.push(Finding {
            class,
            segment,
            position,
            message: message.into(),
        });
    }
}

#[derive(Debug)]
pub enum Error {
    /// Another process holds `LOCK`.
    Locked,
    Io(io::Error),
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Checks every segment in full, and changes no segment.
pub fn verify(dir: impl AsRef<Path>) -> Result<Report, Error> {
    verify_with(
        &storage::real::Real::default(),
        dir.as_ref(),
        codec::FORMAT_VERSION,
    )
}

/// Takes `LOCK` as a writer does, without waiting. Unlike open, it never creates the directory.
fn lock(storage: &dyn Storage, dir: &Path) -> Result<Box<dyn Lock>, Error> {
    let path = dir.join("LOCK");
    match call(storage, &path, Purpose::Open, Op::Create) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e.into()),
        _ => (),
    }
    storage.lock(&path).map_err(|e| match e.kind() {
        io::ErrorKind::WouldBlock => Error::Locked,
        _ => Error::Io(e),
    })
}

pub(crate) fn verify_with(
    storage: &dyn Storage,
    dir: &Path,
    version: u16,
) -> Result<Report, Error> {
    let _lock = lock(storage, dir)?;
    let reader = ReadOnly::new(storage);
    let mut report = Report::default();
    let mut bases = survey::segment_bases(&reader, dir)?;
    if let Some(&newest) = bases.last()
        && survey::is_torn_header(&reader, dir, newest)?
    {
        let message = "the newest segment holds only a torn header; open removes it";
        report.note(Class::TornTail, newest, 0, message);
        bases.pop();
    }
    let mut headers = vec![];
    for &base in &bases {
        match survey::header(&reader, dir, base, version) {
            Ok(header) => headers.push(header),
            Err(OpenError::UnsupportedVersion(v)) => {
                let message = format!("format version {v} is not one this build reads");
                report.note(Class::E, base, 0, message);
            }
            Err(OpenError::Io(e)) => return Err(e.into()),
            Err(_) => report.note(Class::E, base, 0, "the segment header is not valid"),
        }
    }
    let Some(first) = headers.first() else {
        return Ok(report);
    };
    let log_id = codec::LogId::new(first.log_id);
    for (&base, header) in bases.iter().zip(&headers) {
        if header.log_id != *log_id.bytes() {
            let message = "the log id differs from the first segment's";
            report.note(Class::E, base, 0, message);
        }
    }
    // Without every header, later checks would only repeat the same damage.
    if report.damaged() {
        return Ok(report);
    }
    let hash = KeyHash::new(log_id.clone());
    let mut kept = Kept {
        published: Published::default(),
        epoch: 0,
        last_group: None,
    };
    let newest = *bases.last().unwrap();
    // The second-newest segment's missing closing run, which open rebuilds (ADR 0061).
    let mut pending = None;
    for (i, (&base, header)) in bases.iter().zip(&headers).enumerate() {
        let successor = bases.get(i + 1).copied();
        if kept.published.watermark != base {
            let message = format!(
                "the segment starts at offset {base}, but the one before ends at {}",
                kept.published.watermark
            );
            report.note(Class::E, base, 0, message);
            kept.published.watermark = base;
        }
        kept.epoch = kept.epoch.max(header.epoch);
        kept.published.newest.epoch_floor = kept.epoch;
        let path = segment_path(dir, base);
        let length = reader.length(&path, Purpose::Recovery)?;
        let scanned = survey::scan(&reader, &log_id, &hash, base, &path, length, &mut kept);
        let (ending, end) = match scanned {
            Ok(scan) => (scan.ending, scan.end),
            Err(OpenError::Corrupt { position, .. }) => {
                let message = "the closing run passes its checks but differs from the run its \
                               groups give (ADR 0054)";
                report.note(Class::C, base, position, message);
                if let Some(next) = successor {
                    kept.published.watermark = next;
                }
                continue;
            }
            Err(OpenError::Io(e)) => return Err(e.into()),
            Err(other) => return Err(io::Error::other(format!("{other:?}")).into()),
        };
        let behind = closing::sealed_behind(&reader, &path, &log_id, length, end)?;
        let damage = report.findings.len();
        match (successor, ending) {
            (Some(_), Ending::Rolled { .. }) => (),
            (Some(next), Ending::Groups | Ending::Stopped)
                if next == newest && !behind && kept.published.watermark == next =>
            {
                let message = "the closing run is missing or torn; open rebuilds it (ADR 0061)";
                pending = Some(report.findings.len());
                report.note(Class::TornTail, base, end, message);
            }
            (Some(_), Ending::Sealed(run)) => {
                let message = "a seal's closing run in a segment that has a successor";
                report.note(Class::C, base, run.start, message);
            }
            (Some(_), Ending::Groups | Ending::Stopped) => {
                let (class, at, message) = classify(&reader, &log_id, &path, length, end)?;
                report.note(class, base, at, message);
            }
            (None, Ending::Stopped) if behind => {
                let message = "open stops here: a frame that is not valid, before a closing run \
                               whose locator and footer are valid";
                report.note(Class::A, base, end, message);
            }
            (None, Ending::Stopped) => {
                report.note(Class::TornTail, base, end, "open truncates here");
            }
            (None, Ending::Rolled { .. } | Ending::Sealed(_)) => {
                // A roll or seal writes the pending run first, so a closed newest segment rules
                // the rebuild out, and open fails.
                if let Some(index) = pending {
                    let finding = &mut report.findings[index];
                    finding.class = Class::C;
                    finding.message = "the closing run is missing, but the newest segment is \
                                       closed, so no crash explains it"
                        .into();
                }
            }
            (None, Ending::Groups) => (),
        }
        // Damage leaves the scan short, so a gap check after it would only repeat it.
        if let Some(next) = successor
            && report.findings.len() > damage
        {
            kept.published.watermark = next;
        }
    }
    for finding in report.findings.clone() {
        if finding.class != Class::TornTail {
            let frames = frames_around(&reader, &log_id, dir, &finding)?;
            report.frames.extend(frames);
        }
    }
    Ok(report)
}

/// Names the frame that stopped the scan of a segment with a successor at `end`.
fn classify(
    reader: &ReadOnly<'_>,
    log: &codec::LogId,
    path: &Path,
    length: u64,
    end: u64,
) -> io::Result<(Class, u64, &'static str)> {
    let mut at = end;
    while at < length {
        let Some(bytes) = reader.frame(path, Purpose::Recovery, at, length)? else {
            let raw = reader.read(path, Purpose::Recovery, at, 9)?;
            return Ok(by_kind(raw.get(8).copied(), at == end, at));
        };
        match codec::decode(log, &bytes) {
            Err(_) => return Ok(by_kind(Some(bytes[8]), at == end, at)),
            Ok(frame) if at == end && matches!(frame.body, codec::Body::Group { .. }) => {
                let message = "a group frame whose offset or epoch does not follow the one before";
                return Ok((Class::B, at, message));
            }
            Ok(_) => at += bytes.len() as u64,
        }
    }
    Ok((Class::C, end, "the closing run does not have a valid shape"))
}

fn by_kind(kind: Option<u8>, first: bool, at: u64) -> (Class, u64, &'static str) {
    match kind {
        Some(1) => (Class::B, at, "a group frame is not valid"),
        Some(5 | 6) => (Class::D, at, "a key filter or key index frame is not valid"),
        Some(2..=4) => (Class::C, at, "a closing frame is not valid"),
        _ if first => (Class::B, at, "a frame is not valid"),
        _ => (Class::C, at, "a closing frame is not valid"),
    }
}

/// Up to three frames before the finding, the frame at it, and, when the segment's last bytes are
/// a valid locator, that locator and the footer it names.
fn frames_around(
    reader: &ReadOnly<'_>,
    log: &codec::LogId,
    dir: &Path,
    finding: &Finding,
) -> io::Result<Vec<FrameInfo>> {
    let path = segment_path(dir, finding.segment);
    let length = reader.length(&path, Purpose::Recovery)?;
    let info = |position: u64, bytes: &[u8]| frame_info(log, finding.segment, position, bytes);
    let mut before = std::collections::VecDeque::new();
    let mut at = codec::HEADER_LEN as u64;
    while at < finding.position {
        let Some(bytes) = reader.frame(&path, Purpose::Recovery, at, length)? else {
            break;
        };
        if before.len() == 3 {
            before.pop_front();
        }
        before.push_back(info(at, &bytes));
        at += bytes.len() as u64;
    }
    let mut out: Vec<_> = before.into();
    if finding.position >= codec::HEADER_LEN as u64 && finding.position < length {
        let at = finding.position;
        let bytes = match reader.frame(&path, Purpose::Recovery, at, length)? {
            Some(bytes) => bytes,
            None => reader.read(&path, Purpose::Recovery, at, 32)?,
        };
        out.push(info(at, &bytes));
    }
    if let Some(locator_at) = length.checked_sub(codec::LOCATOR_LEN as u64) {
        let locator = reader.read(&path, Purpose::Recovery, locator_at, codec::LOCATOR_LEN)?;
        if let Ok(codec::Body::Locator { footer_position }) =
            codec::decode(log, &locator).map(|f| f.body)
            && footer_position > finding.position
            && let Some(footer) =
                reader.frame(&path, Purpose::Recovery, footer_position, locator_at)?
        {
            out.push(info(footer_position, &footer));
            out.push(info(locator_at, &locator));
        }
    }
    Ok(out)
}

fn frame_info(log: &codec::LogId, segment: u64, position: u64, bytes: &[u8]) -> FrameInfo {
    let field = |range: std::ops::Range<usize>| {
        bytes.get(range).map_or(0, |b| {
            b.iter()
                .rev()
                .fold(0u64, |n, &byte| n << 8 | u64::from(byte))
        })
    };
    let kind = match bytes.get(8) {
        Some(1) => "group",
        Some(2) => "footer",
        Some(3) => "seal",
        Some(4) => "locator",
        Some(5) => "key-filter",
        Some(6) => "key-index",
        _ => "unknown",
    };
    FrameInfo {
        segment,
        position,
        kind,
        epoch: field(12..20),
        base_offset: field(20..28),
        count: field(28..32) as u32,
        valid: codec::decode(log, bytes).is_ok(),
    }
}
