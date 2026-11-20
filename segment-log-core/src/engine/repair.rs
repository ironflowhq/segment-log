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
    /// An unsealed segment whose damage has acknowledged group frames after it (ADR 0068).
    F,
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
            Self::F => "F",
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
    Ok(inspect(storage, dir, version)?.0)
}

/// What a class A or F cut needs from the newest segment's scan.
struct Cut {
    log_id: codec::LogId,
    base: u64,
    length: u64,
    at: u64,
    /// The first log offset after the records that the cut keeps.
    next: u64,
    /// The highest epoch of any header or kept frame.
    epoch: u64,
    tails: BTreeMap<Vec<u8>, closing::Tail>,
    /// For class F, the group frames after the cut that prove it lossy; empty for class A.
    proofs: Vec<survey::Proof>,
    /// Whether the second-newest segment lacks its closing run, which open rebuilds only while
    /// the newest segment is not closed.
    pending_run: bool,
}

/// `verify` without the lock, and the class A cut when there is one.
fn inspect(
    storage: &dyn Storage,
    dir: &Path,
    version: u16,
) -> Result<(Report, Option<Cut>), Error> {
    let reader = ReadOnly::new(storage);
    let mut cut = None;
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
        return Ok((report, None));
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
        return Ok((report, None));
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
        let scan = match scanned {
            Ok(scan) => scan,
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
        let end = scan.end;
        let behind = closing::sealed_behind(&reader, &path, &log_id, length, end)?;
        let damage = report.findings.len();
        match (successor, scan.ending) {
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
            // Open checks for a proof only where it scans groups: here, the second-newest segment.
            (Some(next), Ending::Stopped)
                if next == newest
                    && !behind
                    && !proofs(&reader, &log_id, &path, (length, end), &kept, 1)?.is_empty() =>
            {
                let message = "a frame that is not valid, before acknowledged group frames, in a \
                               segment that has a successor; not repairable (ADR 0068)";
                report.note(Class::F, base, end, message);
            }
            (Some(_), Ending::Groups | Ending::Stopped) => {
                let (class, at, message) = classify(&reader, &log_id, &path, length, end)?;
                report.note(class, base, at, message);
            }
            (None, Ending::Stopped) if behind => {
                let message = "open stops here: a frame that is not valid, before a closing run \
                               whose locator and footer are valid";
                report.note(Class::A, base, end, message);
                cut = Some(Cut {
                    log_id: log_id.clone(),
                    base,
                    length,
                    at: end,
                    next: kept.published.watermark,
                    epoch: kept.epoch,
                    tails: scan.tails,
                    proofs: vec![],
                    pending_run: pending.is_some(),
                });
            }
            (None, Ending::Stopped)
                if let found =
                    proofs(&reader, &log_id, &path, (length, end), &kept, usize::MAX)?
                    && !found.is_empty() =>
            {
                let message = "open stops here: a frame that is not valid, before acknowledged \
                               group frames (ADR 0068)";
                report.note(Class::F, base, end, message);
                cut = Some(Cut {
                    log_id: log_id.clone(),
                    base,
                    length,
                    at: end,
                    next: kept.published.watermark,
                    epoch: kept.epoch,
                    tails: scan.tails,
                    proofs: found,
                    pending_run: pending.is_some(),
                });
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
    Ok((report, cut))
}

fn proofs(
    reader: &ReadOnly<'_>,
    log: &codec::LogId,
    path: &Path,
    (length, end): (u64, u64),
    kept: &Kept,
    limit: usize,
) -> io::Result<Vec<survey::Proof>> {
    survey::proofs(
        reader,
        path,
        log,
        (length, end, kept.published.watermark),
        limit,
    )
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanKind {
    /// No acknowledged record follows the cut. The log stays writable.
    TornTail,
    /// Acknowledged records follow the cut. The repair drops them and seals the log.
    Lossy,
}

impl PlanKind {
    /// The name in the JSON output of api.md.
    pub fn label(self) -> &'static str {
        match self {
            Self::TornTail => "torn-tail",
            Self::Lossy => "lossy",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub segment: LogOffset,
    pub cut: u64,
    pub kind: PlanKind,
    /// The half-open range of log offsets that a lossy plan drops.
    pub dropped_offsets: Option<(LogOffset, LogOffset)>,
    pub closing_frames_removed: Vec<&'static str>,
    pub backup: Option<PathBuf>,
    pub applied: bool,
}

/// A `truncate` run: what `verify` found, the plan for a class A cut, and why the tool refused,
/// if it did.
#[derive(Debug)]
pub struct Outcome {
    pub report: Report,
    pub plan: Option<Plan>,
    pub refusal: Option<String>,
}

/// What `--apply` must match and may do.
pub struct Apply<'a> {
    /// The plan's cut, as `--at` names it: the segment's base offset and the position.
    pub at: (LogOffset, u64),
    pub drop_acknowledged: bool,
    /// The backup directory's name inside the log directory.
    pub backup: &'a str,
}

/// The plan for the damage `verify` finds, changing nothing.
pub fn plan(dir: impl AsRef<Path>) -> Result<Outcome, Error> {
    plan_with(
        &storage::real::Real::default(),
        dir.as_ref(),
        codec::FORMAT_VERSION,
    )
}

/// Carries out the plan, after a backup of the newest segment.
pub fn apply(dir: impl AsRef<Path>, apply: Apply<'_>) -> Result<Outcome, Error> {
    apply_with(
        &storage::real::Real::default(),
        dir.as_ref(),
        codec::FORMAT_VERSION,
        apply,
    )
}

pub(crate) fn plan_with(storage: &dyn Storage, dir: &Path, version: u16) -> Result<Outcome, Error> {
    let _lock = lock(storage, dir)?;
    Ok(planned(storage, dir, version)?.0)
}

/// The cut, and the seal run that a lossy plan writes at it; empty for a torn tail.
type Work = (Cut, Vec<u8>);

/// The plan, and the work `--apply` does when it carries the plan out.
fn planned(
    storage: &dyn Storage,
    dir: &Path,
    version: u16,
) -> Result<(Outcome, Option<Work>), Error> {
    let (report, cut) = inspect(storage, dir, version)?;
    let refuse = |report, refusal: String| {
        let outcome = Outcome {
            report,
            plan: None,
            refusal: Some(refusal),
        };
        Ok((outcome, None))
    };
    // Class F in a segment with a successor has no cut, and is refused here.
    let repairable = |f: &Finding| match f.class {
        Class::A | Class::TornTail => true,
        Class::F => cut.as_ref().is_some_and(|c| c.base == f.segment),
        _ => false,
    };
    if let Some(f) = report.findings.iter().find(|f| !repairable(f)) {
        let refusal = format!(
            "class {} at segment {} position {} is not repairable in 0.1",
            f.class.label(),
            f.segment,
            f.position
        );
        return refuse(report, refusal);
    }
    let Some(cut) = cut else {
        return refuse(
            report,
            "no class A or F finding: nothing to truncate".into(),
        );
    };
    if let Some(last) = cut.proofs.last() {
        // A seal run closes the newest segment, and open rebuilds a missing run only behind an
        // open newest segment (ADR 0061), so the log would never open again.
        if cut.pending_run {
            let refusal = "the second-newest segment lacks its closing run, and sealing the newest \
                           segment would leave it missing; this case stays manual";
            return refuse(report, refusal.into());
        }
        let dropped_end = cut.proofs.iter().map(|p| p.end).max().unwrap_or(cut.next);
        let plan = Plan {
            segment: cut.base,
            cut: cut.at,
            kind: PlanKind::Lossy,
            dropped_offsets: Some((cut.next, dropped_end.max(cut.next))),
            closing_frames_removed: vec![],
            backup: None,
            applied: false,
        };
        // Until the truncation, the last proof must stay whole, so that open still refuses and
        // every crash state fails closed with `Corrupt` (ADR 0068).
        let room = last.position;
        return sealed(report, plan, cut, room, "the last acknowledged group frame");
    }
    // `verify` found this locator and footer valid.
    let reader = ReadOnly::new(storage);
    let path = segment_path(dir, cut.base);
    let locator_at = cut.length - codec::LOCATOR_LEN as u64;
    let locator = reader.read(&path, Purpose::Repair, locator_at, codec::LOCATOR_LEN)?;
    let Ok(codec::Body::Locator { footer_position }) =
        codec::decode(&cut.log_id, &locator).map(|f| f.body)
    else {
        return Err(io::Error::other("the locator changed under the lock").into());
    };
    let footer = reader
        .frame(&path, Purpose::Repair, footer_position, locator_at)?
        .ok_or_else(|| io::Error::other("the footer changed under the lock"))?;
    let Ok(codec::Frame {
        base_offset: footer_next,
        body: codec::Body::Footer(footer),
        ..
    }) = codec::decode(&cut.log_id, &footer)
    else {
        return Err(io::Error::other("the footer changed under the lock").into());
    };
    // The footer names the segment's last group frame for each of its streams, so any group
    // after the cut makes the plan lossy. A lookalike footer can only err toward lossy.
    let lossy = footer.streams.iter().any(|t| t.last_position >= cut.at);
    let closing_frames_removed = match footer.key_filter_position {
        Some(_) => vec!["key-filter", "key-index", "footer", "locator"],
        None => vec!["footer", "seal", "locator"],
    };
    let mut plan = Plan {
        segment: cut.base,
        cut: cut.at,
        kind: PlanKind::TornTail,
        dropped_offsets: None,
        closing_frames_removed,
        backup: None,
        applied: false,
    };
    if !lossy {
        let outcome = Outcome {
            report,
            plan: Some(plan),
            refusal: None,
        };
        return Ok((outcome, Some((cut, vec![]))));
    }
    plan.kind = PlanKind::Lossy;
    plan.dropped_offsets = Some((cut.next, footer_next.max(cut.next)));
    // Until the truncation, the old footer and locator must stay whole, so that open still proves
    // the old run and every crash state fails closed with `Corrupt` (ADR 0065).
    sealed(report, plan, cut, footer_position, "the old footer")
}

/// A lossy plan, and its seal run unless the run would reach `room`, which must survive until the
/// truncation.
fn sealed(
    report: Report,
    plan: Plan,
    cut: Cut,
    room: u64,
    guard: &str,
) -> Result<(Outcome, Option<Work>), Error> {
    let mut outcome = Outcome {
        report,
        plan: Some(plan),
        refusal: None,
    };
    let Some(epoch) = cut.epoch.checked_add(1) else {
        outcome.refusal = Some("the highest epoch cannot be incremented for the seal run".into());
        return Ok((outcome, None));
    };
    let run = closing::seal_run(&cut.log_id, epoch, cut.next, cut.at, &cut.tails).concat();
    if cut.at + run.len() as u64 > room {
        let refusal = format!("the seal run would reach {guard}; this case stays manual");
        outcome.refusal = Some(refusal);
        return Ok((outcome, None));
    }
    Ok((outcome, Some((cut, run))))
}

pub(crate) fn apply_with(
    storage: &dyn Storage,
    dir: &Path,
    version: u16,
    apply: Apply<'_>,
) -> Result<Outcome, Error> {
    let _lock = lock(storage, dir)?;
    let (mut outcome, cut) = planned(storage, dir, version)?;
    let (Some(plan), Some((cut, run))) = (&mut outcome.plan, cut) else {
        return Ok(outcome);
    };
    if apply.at != (plan.segment, plan.cut) {
        let refusal = format!(
            "--at {}:{} differs from the plan's cut {}:{}",
            apply.at.0, apply.at.1, plan.segment, plan.cut
        );
        outcome.refusal = Some(refusal);
        return Ok(outcome);
    }
    if plan.kind == PlanKind::Lossy && !apply.drop_acknowledged {
        let refusal = "the plan drops acknowledged records; give --drop-acknowledged";
        outcome.refusal = Some(refusal.into());
        return Ok(outcome);
    }
    let segment = segment_path(dir, cut.base);
    let backup = dir.join(apply.backup);
    copy_durably(storage, &segment, &backup, cut.length)?;
    call(storage, dir, Purpose::Repair, Op::SyncDir)?;
    plan.backup = Some(backup);
    if run.is_empty() {
        call(storage, &segment, Purpose::Repair, Op::Truncate(cut.at))?;
        call(storage, &segment, Purpose::Repair, Op::SyncAll)?;
        call(storage, dir, Purpose::Repair, Op::SyncDir)?;
    } else {
        // The run lands over the damage before the truncation, so the old locator ends the file
        // until the run is durable.
        let end = cut.at + run.len() as u64;
        call(storage, &segment, Purpose::Repair, Op::Write(cut.at, run))?;
        call(storage, &segment, Purpose::Repair, Op::SyncAll)?;
        call(storage, &segment, Purpose::Repair, Op::Truncate(end))?;
        call(storage, &segment, Purpose::Repair, Op::SyncAll)?;
    }
    plan.applied = true;
    Ok(outcome)
}

/// Copies the segment byte for byte into the backup directory, and returns once the copy and its
/// directory entry are durable.
fn copy_durably(
    storage: &dyn Storage,
    segment: &Path,
    backup: &Path,
    length: u64,
) -> io::Result<()> {
    const CHUNK: u64 = 1 << 20;
    call(storage, backup, Purpose::Repair, Op::CreateDir)?;
    let copy = backup.join(segment.file_name().expect("a segment path names a file"));
    call(storage, &copy, Purpose::Repair, Op::Create)?;
    let reader = ReadOnly::new(storage);
    let mut at = 0;
    while at < length {
        let len = CHUNK.min(length - at) as usize;
        let bytes = reader.read(segment, Purpose::Repair, at, len)?;
        if bytes.len() != len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        call(storage, &copy, Purpose::Repair, Op::Write(at, bytes))?;
        at += len as u64;
    }
    call(storage, &copy, Purpose::Repair, Op::SyncAll)?;
    call(storage, backup, Purpose::Repair, Op::SyncDir)?;
    Ok(())
}
