#![cfg(unix)]

#[path = "../src/acceptance/support/deadline.rs"]
mod deadline;
#[path = "../src/acceptance/support/roll_size.rs"]
mod roll_size;

use segment_log_core::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const PREFIX: &str = "SL8 ";
const CHILD_TEST: &str = "killed_writers_preserve_the_recorded_contract";
const LIMIT: Duration = Duration::from_secs(20);

fn options() -> Options {
    Options {
        linger: Duration::ZERO,
        roll_bytes: roll_size::roll_bytes(Options::default().roll_bytes),
        ..Options::default()
    }
}

struct Scenario {
    root: PathBuf,
    child: Arc<Mutex<Option<Child>>>,
    deadline: Option<deadline::Deadline>,
}

fn stop(child: &mut Option<Child>) {
    if let Some(c) = child {
        let _ = c.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match c.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::yield_now(),
                _ => {
                    std::process::abort();
                }
            }
        }
    }
    *child = None;
}

impl Scenario {
    fn new(mode: &str, seed: u64) -> Self {
        let root = std::env::temp_dir().join(format!(
            "segment-log-slice8-{}-{mode}-{seed}",
            std::process::id()
        ));
        let child = Arc::new(Mutex::new(None));
        let owned = child.clone();
        let message = format!(
            "seed={seed} mode={mode} transcript={}",
            root.join("protocol.txt").display()
        );
        let initial = message.clone();
        let watchdog = deadline::Deadline::new(LIMIT, move || {
            stop(&mut owned.lock().unwrap_or_else(|e| e.into_inner()));
            eprintln!("scenario watchdog: {message}");
        });
        fs::create_dir(&root).unwrap();
        eprintln!("{initial}");
        Self {
            root,
            child,
            deadline: Some(watchdog),
        }
    }
    fn log(&self) -> PathBuf {
        self.root.join("log")
    }
}
impl Drop for Scenario {
    fn drop(&mut self) {
        stop(&mut self.child.lock().unwrap_or_else(|e| e.into_inner()));
        if !thread::panicking() {
            fs::remove_dir_all(&self.root).unwrap();
        }
        drop(self.deadline.take());
    }
}

/// Given a real directory with a live writer, missing ancestors, or no write permission
/// When invalid options are passed, and then a valid tiny budget is used on a fresh log
/// Then validation precedes filesystem access without mutations, and only the append is TooLarge
/// Cases: OPT-VALIDATION, OPEN-DIR
#[test]
fn public_validation_precedes_filesystem_access() {
    let s = Scenario::new("options", 0);
    let live = SegmentLog::open(s.log(), options()).unwrap();
    let denied = s.root.join("denied");
    fs::create_dir(&denied).unwrap();
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o555)).unwrap();
    for path in [s.log(), s.root.join("missing/log"), denied.join("log")] {
        let before = snapshot(&s.root);
        assert!(matches!(
            SegmentLog::open(
                &path,
                Options {
                    queue_capacity: 0,
                    ..options()
                }
            ),
            Err(OpenError::InvalidOptions(_))
        ));
        assert_eq!(snapshot(&s.root), before);
    }
    fs::set_permissions(denied, fs::Permissions::from_mode(0o755)).unwrap();
    drop(live);
    let log = SegmentLog::open(
        s.log(),
        Options {
            pending_byte_budget: 1,
            ..options()
        },
    )
    .unwrap();
    assert!(matches!(
        log.append(Submit {
            records: vec![input(b"s", b"key", b"payload", ExpectedRevision::Empty)]
        }),
        Err(AppendError::TooLarge)
    ));
    assert_eq!(log.watermark(), 0);
    drop(log);
    assert!(matches!(
        SegmentLog::open(s.root.join("absent/log"), options()),
        Err(OpenError::Io(_))
    ));
    assert!(!s.root.join("absent").exists());
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.insert(path.clone(), vec![]);
            out.extend(snapshot(&path));
        } else {
            out.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    out
}

/// Given a live public handle and then a second segment longer than a header with a bad header
/// When another handle opens, Drop runs, open fails, or the owner unwinds
/// Then the live writer excludes competitors and every completed owner releases the OS lock
/// And the failed open changes no file
/// Cases: ORACLE-11, OPEN-LAYOUT, RECOVERY-FIXTURES
#[test]
fn public_locks_release_after_drop_failed_open_and_unwind() {
    let s = Scenario::new("locks", 0);
    let log = SegmentLog::open(s.log(), options()).unwrap();
    assert!(matches!(
        SegmentLog::open(s.log(), options()),
        Err(OpenError::Locked)
    ));
    drop(log);
    let extra = s.log().join("00000000000000000001.seg");
    fs::write(&extra, [0xff; 65]).unwrap();
    let before = snapshot(&s.root);
    for _ in 0..2 {
        assert!(matches!(
            SegmentLog::open(s.log(), options()),
            Err(OpenError::Corrupt {
                segment: 1,
                position: 0
            })
        ));
        assert_eq!(snapshot(&s.root), before);
    }
    fs::remove_file(extra).unwrap();
    let path = s.log();
    assert!(
        std::panic::catch_unwind(|| {
            let _log = SegmentLog::open(&path, options()).unwrap();
            panic!("owner unwinds");
        })
        .is_err()
    );
    drop(SegmentLog::open(s.log(), options()).unwrap());
}

fn input(stream: &[u8], key: &[u8], payload: &[u8], expected: ExpectedRevision) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        key: Some(key.to_vec()),
        payload: payload.to_vec(),
        expected,
    }
}

#[derive(Debug)]
struct Request {
    id: u64,
    records: Vec<NewRecord>,
}

#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Ack {
        deduplicated: bool,
        positions: Vec<(u64, u64)>,
    },
    Wrong {
        index: usize,
        actual: Option<u64>,
    },
    Conflict(usize),
    Partial,
    Full,
    Unknown,
}

fn execute(log: &SegmentLog, request: &Request) -> Answer {
    match log.append(Submit {
        records: request.records.iter().map(copy_record).collect(),
    }) {
        Ok(pending) => match pending.wait() {
            Ok(c) => Answer::Ack {
                deduplicated: c.deduplicated,
                positions: c.records.iter().map(|p| (p.offset, p.revision)).collect(),
            },
            Err(CommitError::WrongRevision { index, actual }) => Answer::Wrong { index, actual },
            Err(CommitError::KeyConflict { index }) => Answer::Conflict(index),
            Err(CommitError::PartialRetry) => Answer::Partial,
            Err(CommitError::Indeterminate) => Answer::Unknown,
            other => panic!("unexpected workload result: {other:?}"),
        },
        Err(AppendError::Full) => Answer::Full,
        Err(other) => panic!("unexpected admission: {other:?}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn expectation(e: &ExpectedRevision) -> String {
    match e {
        ExpectedRevision::Empty => "e".into(),
        ExpectedRevision::At(n) => n.to_string(),
    }
}
fn emit(line: &str) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "\n{PREFIX}{line}").unwrap();
    stdout.flush().unwrap();
}
fn request_line(r: &Request) -> String {
    let fields: Vec<_> = r
        .records
        .iter()
        .map(|r| {
            format!(
                "{},{},{},{}",
                hex(&r.stream),
                expectation(&r.expected),
                hex(r.key.as_ref().unwrap()),
                hex(&r.payload)
            )
        })
        .collect();
    format!("Q {} {}", r.id, fields.join(" "))
}
fn answer_line(id: u64, answer: &Answer) -> String {
    let result = match answer {
        Answer::Ack {
            deduplicated,
            positions,
        } => format!(
            "A {} {}",
            u8::from(*deduplicated),
            positions
                .iter()
                .map(|(o, r)| format!("{o},{r}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Answer::Wrong { index, actual } => format!(
            "R {index} {}",
            actual.map_or_else(|| "e".into(), |n| n.to_string())
        ),
        Answer::Conflict(i) => format!("C {i}"),
        Answer::Partial => "P".into(),
        Answer::Full => "F".into(),
        Answer::Unknown => "U".into(),
    };
    format!("D {id} {result}")
}

fn literal(lane: u64, batch: u64, index: u64) -> NewRecord {
    let revision = batch * 2 + index;
    input(
        format!("lane-{lane}").as_bytes(),
        format!("key-{lane}-{batch}-{index}").as_bytes(),
        &[lane as u8, (batch % 256) as u8, index as u8, 0, 255],
        if revision == 0 {
            ExpectedRevision::Empty
        } else {
            ExpectedRevision::At(revision - 1)
        },
    )
}

fn child_workload(dir: PathBuf) {
    let log = Arc::new(SegmentLog::open(dir, options()).unwrap());
    emit("READY");
    let mut workers = vec![];
    for lane in 0..6 {
        let log = log.clone();
        workers.push(thread::spawn(move || {
            let mut serial = 0;
            let mut send = |records| {
                let request = Request {
                    id: serial * 6 + lane,
                    records,
                };
                serial += 1;
                emit(&request_line(&request));
                let answer = if request.id == 0 {
                    drop(
                        log.append(Submit {
                            records: request.records.iter().map(copy_record).collect(),
                        })
                        .unwrap(),
                    );
                    emit("B 0");
                    Answer::Unknown
                } else {
                    let answer = execute(&log, &request);
                    emit(&answer_line(request.id, &answer));
                    answer
                };
                emit(&format!("W {}", log.watermark()));
                answer
            };
            for batch in 0.. {
                let original = [literal(lane, batch, 0), literal(lane, batch, 1)];
                let answer = send(original.iter().map(copy_record).collect());
                assert!(
                    matches!(
                        answer,
                        Answer::Ack {
                            deduplicated: false,
                            ..
                        }
                    ) || (lane == 0 && batch == 0 && answer == Answer::Unknown)
                );
                let mut regrouped = vec![copy_record(&original[1]), copy_record(&original[0])];
                if batch > 0 {
                    regrouped.push(literal(lane, batch - 1, 0));
                }
                send(regrouped);
                send(vec![input(
                    format!("lane-{lane}").as_bytes(),
                    format!("wrong-{lane}-{batch}").as_bytes(),
                    b"refused",
                    ExpectedRevision::Empty,
                )]);
                send(vec![
                    copy_record(&original[0]),
                    input(
                        b"fresh",
                        format!("partial-{lane}-{batch}").as_bytes(),
                        b"absent",
                        ExpectedRevision::Empty,
                    ),
                ]);
                let mut conflict = copy_record(&original[0]);
                conflict.payload = b"conflict".to_vec();
                send(vec![conflict]);
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    panic!("workload must remain busy until killed");
}

fn copy_record(r: &NewRecord) -> NewRecord {
    NewRecord {
        stream: r.stream.clone(),
        key: r.key.clone(),
        payload: r.payload.clone(),
        expected: match r.expected {
            ExpectedRevision::Empty => ExpectedRevision::Empty,
            ExpectedRevision::At(n) => ExpectedRevision::At(n),
        },
    }
}

#[derive(Default)]
struct Ledger {
    requests: BTreeMap<u64, Request>,
    answers: BTreeMap<u64, Answer>,
    watermark: u64,
    ready: bool,
    abandoned: bool,
}
impl Ledger {
    fn line(&mut self, line: &str) {
        let Some(line) = line.strip_prefix(PREFIX) else {
            return;
        };
        let fields: Vec<_> = line.split_whitespace().collect();
        match fields[0] {
            "READY" => self.ready = true,
            "B" => {
                assert_eq!(fields.as_slice(), ["B", "0"]);
                assert!(self.requests.contains_key(&0));
                self.abandoned = true;
            }
            "W" => self.watermark = self.watermark.max(fields[1].parse().unwrap()),
            "Q" => {
                let id = fields[1].parse().unwrap();
                let records = fields[2..]
                    .iter()
                    .map(|field| {
                        let p: Vec<_> = field.split(',').collect();
                        assert_eq!(p.len(), 4);
                        input(
                            &unhex(p[0]),
                            &unhex(p[2]),
                            &unhex(p[3]),
                            if p[1] == "e" {
                                ExpectedRevision::Empty
                            } else {
                                ExpectedRevision::At(p[1].parse().unwrap())
                            },
                        )
                    })
                    .collect();
                assert!(self.requests.insert(id, Request { id, records }).is_none());
            }
            "D" => {
                let id = fields[1].parse().unwrap();
                assert!(self.requests.contains_key(&id));
                let answer = match fields[2] {
                    "A" => Answer::Ack {
                        deduplicated: match fields[3] {
                            "0" => false,
                            "1" => true,
                            _ => panic!("bad ack"),
                        },
                        positions: fields[4..]
                            .iter()
                            .map(|p| {
                                let (o, r) = p.split_once(',').unwrap();
                                (o.parse().unwrap(), r.parse().unwrap())
                            })
                            .collect(),
                    },
                    "R" => Answer::Wrong {
                        index: fields[3].parse().unwrap(),
                        actual: if fields[4] == "e" {
                            None
                        } else {
                            Some(fields[4].parse().unwrap())
                        },
                    },
                    "C" => Answer::Conflict(fields[3].parse().unwrap()),
                    "P" => Answer::Partial,
                    "F" => Answer::Full,
                    "U" => Answer::Unknown,
                    _ => panic!("bad reply"),
                };
                assert!(self.answers.insert(id, answer).is_none());
            }
            _ => panic!("bad protocol line {line}"),
        }
    }
}

fn assert_history(log: &SegmentLog, expected: &[Record]) {
    let actual = log.read(0, usize::MAX).unwrap();
    assert_eq!(actual.len(), expected.len());
    assert_eq!(log.watermark(), expected.len() as u64);
    for (a, e) in actual.iter().zip(expected) {
        assert_eq!(
            (a.offset, a.revision, &a.stream, &a.key, &a.payload),
            (e.offset, e.revision, &e.stream, &e.key, &e.payload)
        );
    }
}

fn predict(records: &[NewRecord], history: &mut Vec<Record>) -> Answer {
    let mut matches = vec![];
    for (index, r) in records.iter().enumerate() {
        if let Some(old) = history.iter().find(|old| old.key == r.key) {
            if old.stream != r.stream || old.payload != r.payload {
                return Answer::Conflict(index);
            }
            matches.push((old.offset, old.revision));
        }
    }
    if matches.len() == records.len() {
        return Answer::Ack {
            deduplicated: true,
            positions: matches,
        };
    }
    if !matches.is_empty() {
        return Answer::Partial;
    }
    let mut heads: BTreeMap<_, _> = history
        .iter()
        .map(|r| (r.stream.clone(), r.revision))
        .collect();
    let mut positions = vec![];
    for (index, r) in records.iter().enumerate() {
        let actual = heads.get(&r.stream).copied();
        let expected = match r.expected {
            ExpectedRevision::Empty => None,
            ExpectedRevision::At(n) => Some(n),
        };
        if actual != expected {
            return Answer::Wrong { index, actual };
        }
        let revision = actual.map_or(0, |n| n + 1);
        heads.insert(r.stream.clone(), revision);
        positions.push((history.len() as u64 + index as u64, revision));
    }
    for (r, &(offset, revision)) in records.iter().zip(&positions) {
        history.push(Record {
            offset,
            revision,
            stream: r.stream.clone(),
            key: r.key.clone(),
            payload: r.payload.clone(),
        });
    }
    Answer::Ack {
        deduplicated: false,
        positions,
    }
}

// The pipe cannot observe writer order across callers. Read the pre-recovery files
// independently to constrain that order; expected records still come from requests.
fn written_keys(dir: &Path) -> Vec<Vec<Vec<u8>>> {
    let mut segments: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "seg"))
        .collect();
    segments.sort();
    let mut frames = vec![];
    for segment in segments {
        frames.extend(segment_keys(&fs::read(segment).unwrap()));
    }
    frames
}

fn segment_keys(bytes: &[u8]) -> Vec<Vec<Vec<u8>>> {
    let mut frames = vec![];
    let mut at = 64;
    while at + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        if !(24..=8 * 1024 * 1024).contains(&len) || at + 8 + len > bytes.len() {
            break;
        }
        let body = &bytes[at + 8..at + 8 + len];
        let mut crc = u32::MAX;
        for b in bytes[at..at + 4].iter().chain(body) {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = (crc >> 1) ^ if crc & 1 == 1 { 0x82f63b78 } else { 0 };
            }
        }
        if !crc != u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) {
            break;
        }
        assert_eq!(body[0], 1);
        let count = u32::from_le_bytes(body[20..24].try_into().unwrap());
        let mut pos = 24;
        let mut keys = vec![];
        for _ in 0..count {
            let stream_len = u16::from_le_bytes(body[pos..pos + 2].try_into().unwrap()) as usize;
            let key_len = u16::from_le_bytes(body[pos + 2..pos + 4].try_into().unwrap()) as usize;
            let payload_len =
                u32::from_le_bytes(body[pos + 4..pos + 8].try_into().unwrap()) as usize;
            let key_start = pos + 16 + stream_len;
            keys.push(body[key_start..key_start + key_len].to_vec());
            pos = key_start + key_len + payload_len;
        }
        assert_eq!(pos, body.len());
        frames.push(keys);
        at += 8 + len;
    }
    frames
}

fn oracle(dir: &Path, ledger: &Ledger) {
    let frames = written_keys(dir);
    let mut originals = BTreeMap::new();
    for request in ledger.requests.values() {
        // Each lane sends five operations per batch, with only the first creating records.
        if (request.id / 6) % 5 == 0 {
            for (index, record) in request.records.iter().enumerate() {
                assert!(
                    originals
                        .insert(record.key.clone().unwrap(), (request, index))
                        .is_none()
                );
            }
        }
    }
    let mut history = vec![];
    let mut seen = BTreeSet::new();
    for frame in frames {
        let mut at = 0;
        while at < frame.len() {
            let (request, index) = originals
                .get(&frame[at])
                .expect("written key had no original request");
            assert_eq!(*index, 0, "new submit cannot be split or reordered");
            assert!(seen.insert(request.id), "no duplicated original");
            let keys: Vec<_> = request
                .records
                .iter()
                .map(|r| r.key.clone().unwrap())
                .collect();
            assert_eq!(
                &frame[at..at + keys.len()],
                keys.as_slice(),
                "atomic original in one frame"
            );
            assert!(
                matches!(
                    predict(&request.records, &mut history),
                    Answer::Ack {
                        deduplicated: false,
                        ..
                    }
                ),
                "written order must respect literal expectations"
            );
            at += keys.len();
        }
    }
    let log = SegmentLog::open(dir, options()).unwrap();
    assert_history(&log, &history);
    assert!(ledger.watermark > 0, "watermark oracle must not be vacuous");
    assert!(log.watermark() >= ledger.watermark);
    let mut acks = 0;
    let mut retries = 0;
    let mut refusals = 0;
    for (id, answer) in &ledger.answers {
        let request = &ledger.requests[id];
        match answer {
            Answer::Ack {
                deduplicated,
                positions,
            } => {
                acks += 1;
                assert_eq!(*deduplicated, (id / 6) % 5 == 1);
                retries += usize::from(*deduplicated);
                assert_eq!(positions.len(), request.records.len());
                for (r, &(offset, revision)) in request.records.iter().zip(positions) {
                    let old = &history[offset as usize];
                    assert_eq!(
                        (&old.stream, &old.key, &old.payload, old.revision),
                        (&r.stream, &r.key, &r.payload, revision)
                    );
                    assert!(offset < log.watermark());
                }
                if !deduplicated {
                    assert!(seen.contains(id));
                }
            }
            Answer::Unknown => (),
            _ => {
                refusals += 1;
                let batch = (id / 6) / 5;
                match (id / 6) % 5 {
                    2 => assert_eq!(
                        *answer,
                        Answer::Wrong {
                            index: 0,
                            actual: Some(batch * 2 + 1)
                        }
                    ),
                    3 => assert_eq!(*answer, Answer::Partial),
                    4 => assert_eq!(*answer, Answer::Conflict(0)),
                    _ => assert_eq!(*answer, Answer::Full),
                }
                assert!(!seen.contains(id), "known refusal cannot create records");
                for r in &request.records {
                    if !originals.contains_key(r.key.as_ref().unwrap()) {
                        assert!(!history.iter().any(|old| old.key == r.key));
                    }
                }
            }
        }
    }
    assert!(
        acks > 0 && retries > 0 && refusals > 0,
        "exercise acknowledgments, regrouped retries and terminal refusals"
    );
    let mut unknown = 0;
    for request in ledger.requests.values() {
        if ledger
            .answers
            .get(&request.id)
            .is_none_or(|a| *a == Answer::Unknown)
        {
            unknown += 1;
            let expected = predict(&request.records, &mut history);
            assert_eq!(
                execute(&log, request),
                expected,
                "unknown request {}",
                request.id
            );
            assert_history(&log, &history);
        }
    }
    assert!(
        unknown > 0,
        "abandoned reply must exercise unknown outcomes"
    );
    let fresh = Request {
        id: u64::MAX,
        records: vec![input(
            b"after-reopen",
            b"after-reopen",
            b"new",
            ExpectedRevision::Empty,
        )],
    };
    let prior = history.len() as u64;
    assert!(prior >= ledger.watermark);
    let expected = predict(&fresh.records, &mut history);
    assert_eq!(execute(&log, &fresh), expected);
    assert_history(&log, &history);
}

struct Pipe {
    lines: mpsc::Receiver<String>,
    reader: Option<JoinHandle<()>>,
    child: Arc<Mutex<Option<Child>>>,
}
impl Drop for Pipe {
    fn drop(&mut self) {
        stop(&mut self.child.lock().unwrap_or_else(|e| e.into_inner()));
        let reader = self.reader.take().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !reader.is_finished() {
            if Instant::now() >= deadline {
                std::process::abort();
            }
            thread::yield_now();
        }
        reader.join().unwrap();
    }
}
fn start(s: &Scenario) -> Pipe {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
        .env("SEGMENT_LOG_PROCESS_CHILD", "workload")
        .env("SEGMENT_LOG_PROCESS_DIR", s.log())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    *s.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    let transcript = File::create(s.root.join("protocol.txt")).unwrap();
    let (tx, lines) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut transcript = transcript;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            transcript.write_all(line.as_bytes()).unwrap();
            transcript.flush().unwrap();
            // A kill may interrupt a protocol write. A partial reply is unknown.
            if line.ends_with('\n') && tx.send(line).is_err() {
                break;
            }
        }
    });
    Pipe {
        lines,
        reader: Some(reader),
        child: s.child.clone(),
    }
}

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name).map_or(default, |s| {
        s.parse().expect("numeric process test setting")
    })
}
/// Whether the oracle gets a known-bad ledger. A hosted drill sets it to prove that a failed
/// repetition keeps a directory that can be fetched and checked again.
fn drill() -> bool {
    match std::env::var("SEGMENT_LOG_PROCESS_DRILL").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("0") => false,
        Ok("1") => true,
        other => panic!("invalid SEGMENT_LOG_PROCESS_DRILL: {other:?}"),
    }
}
/// Checks a ledger against a log. The drill first claims a watermark that no child reported.
fn check(dir: &Path, mut ledger: Ledger) {
    if drill() {
        ledger.watermark = u64::MAX;
    }
    oracle(dir, &ledger);
}
/// Rebuilds the ledger of a kept repetition from its transcript and checks its log again.
/// The first check already recovered that log, and its retries may have appended to it.
fn replay(root: &Path) {
    let mut ledger = Ledger::default();
    let transcript = fs::read_to_string(root.join("protocol.txt")).unwrap();
    // A kill can cut the last line. The pipe reader never forwarded such a line either.
    for line in transcript.split_inclusive('\n') {
        if line.ends_with('\n') {
            ledger.line(line);
        }
    }
    check(&root.join("log"), ledger);
}
fn mix(mut seed: u64) -> u64 {
    seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    seed = (seed ^ (seed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    seed = (seed ^ (seed >> 27)).wrapping_mul(0x94d049bb133111eb);
    seed ^ (seed >> 31)
}

/// Given six caller threads logging literal requests before append and results after completion
/// And one admitted result handle is abandoned without observing or reporting its answer
/// When a seeded request-event boundary triggers SIGKILL during their continuing workload
/// Then a second process is excluded while live, the killed writer releases its lock,
/// And recovery preserves acknowledged identities, atomic written prefixes and watermarks
/// And unknown fully keyed requests follow predicted retry classification without reusing durable offsets
/// Cases: PROC-SIGKILL, ORACLE-1, ORACLE-2, ORACLE-3, ORACLE-4, ORACLE-9, ORACLE-10, ORACLE-11
#[test]
fn killed_writers_preserve_the_recorded_contract() {
    if let Ok(mode) = std::env::var("SEGMENT_LOG_PROCESS_CHILD") {
        assert_eq!(mode, "workload");
        child_workload(std::env::var_os("SEGMENT_LOG_PROCESS_DIR").unwrap().into());
        return;
    }
    if let Some(root) = std::env::var_os("SEGMENT_LOG_PROCESS_REPLAY") {
        replay(Path::new(&root));
        return;
    }
    let repetitions = setting("SEGMENT_LOG_PROCESS_REPETITIONS", 3);
    assert!((1..=10000).contains(&repetitions));
    let seed = setting("SEGMENT_LOG_PROCESS_SEED", 808);
    for repetition in 0..repetitions {
        let seed = seed.wrapping_add(repetition);
        let s = Scenario::new("SIGKILL", seed);
        let pipe = start(&s);
        let target = 100 + (mix(seed) % 100) as usize;
        let mut ledger = Ledger::default();
        let mut locked = false;
        loop {
            let line = pipe
                .lines
                .recv_timeout(Duration::from_secs(5))
                .expect("workload pipe watchdog or unexpected child exit");
            ledger.line(&line);
            if ledger.ready && !locked {
                assert!(matches!(
                    SegmentLog::open(s.log(), options()),
                    Err(OpenError::Locked)
                ));
                locked = true;
            }
            if locked
                && ledger.abandoned
                && ledger.requests.len() >= target
                && ledger.watermark > 0
                && line.starts_with("SL8 Q ")
            {
                let mut child = s.child.lock().unwrap_or_else(|e| e.into_inner());
                let child = child.as_mut().unwrap();
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "child exited before kill"
                );
                child.kill().unwrap();
                break;
            }
        }
        loop {
            match pipe.lines.recv_timeout(Duration::from_secs(5)) {
                Ok(line) => ledger.line(&line),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(e) => panic!("killed pipe watchdog: {e}"),
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = s
                .child
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
            {
                assert_eq!(status.signal(), Some(9));
                break;
            }
            assert!(Instant::now() < deadline, "child exit watchdog");
            thread::yield_now();
        }
        drop(pipe);
        check(&s.log(), ledger);
    }
}
