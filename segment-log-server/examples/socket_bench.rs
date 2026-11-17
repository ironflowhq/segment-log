//! The M6 socket diagnostic (ADR 0056): the M1 append workload, run in process or over the Unix
//! socket, so that the difference between two runs is the transport.
//!
//! usage: socket_bench (--inproc <log-dir> | --unix <socket> | --firecracker <uds>:<port>) <callers> <rounds> [<interval-us>] [--connections <n>]
//!
//! `--firecracker` dials a server in a Firecracker guest through the VMM's hybrid vsock socket
//! (`CONNECT <port>`), the caller's path on the M7 stack (ADR 0059): the M7 guest benchmark.
//!
//! Each caller appends one keyed 256-byte record per round to its own stream, expecting the
//! previous revision, as the M1 benchmark does (segment-log-core/src/acceptance/evidence.rs).
//! Without an interval each caller submits again as soon as it is answered (a closed loop). With
//! one, each caller submits once per interval, the callers spread evenly over it, and latency runs
//! from the scheduled send (ADR 0045). Over the socket each caller has its own connection, so the
//! server must allow that many (`--max-connections`). With `--connections <n>` the callers share
//! `n` pipelined connections instead, caller `i` on connection `i % n`: each connection sends the
//! requests queued together in one write, and hands the answers back in request order.

use segment_log_core::{AppendError, ExpectedRevision, NewRecord, Options, SegmentLog, Submit};
use segment_log_server::protocol::{self, HEADER_LEN, Request};
use std::io::BufReader;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

fn input(caller: usize, revision: usize) -> Submit {
    let mut payload = vec![0x5a; 256];
    payload[..8].copy_from_slice(&(caller as u64).to_le_bytes());
    payload[8..16].copy_from_slice(&(revision as u64).to_le_bytes());
    Submit {
        records: vec![NewRecord {
            stream: (caller as u64).to_le_bytes().to_vec(),
            key: Some(payload[..16].to_vec()),
            payload,
            expected: match revision {
                0 => ExpectedRevision::Empty,
                n => ExpectedRevision::At(n as u64 - 1),
            },
        }],
    }
}

/// One caller's way to append: the outcome is the committed revision, or `None` for `Full`. A
/// deduplicated answer panics: it means the log was not fresh, and the run measured no syncs.
enum Target {
    InProcess(Arc<SegmentLog>),
    Socket(UnixStream),
    Shared(Sender<Exchange>),
}

/// A request's bytes, and where its connection sends the answer's body.
type Exchange = (Vec<u8>, Sender<Vec<u8>>);

/// Starts a pipelined connection's two threads and returns the way to send it requests.
fn share(stream: UnixStream) -> Sender<Exchange> {
    let (requests, queue) = mpsc::channel::<Exchange>();
    let (waiting, answers) = mpsc::channel::<Sender<Vec<u8>>>();
    let mut writer = stream.try_clone().unwrap();
    thread::spawn(move || {
        while let Ok(first) = queue.recv() {
            let mut bytes = Vec::new();
            for (request, reply) in std::iter::once(first).chain(queue.try_iter()) {
                bytes.extend(request);
                // Queued before the write, so the reader always finds the request's caller.
                waiting.send(reply).unwrap();
            }
            writer.write_all(&bytes).unwrap();
        }
    });
    thread::spawn(move || deliver(BufReader::new(stream), answers));
    requests
}

/// Hands each answer's body to the caller of the oldest unanswered request.
fn deliver(mut stream: impl Read, answers: Receiver<Sender<Vec<u8>>>) {
    for reply in answers {
        let mut header = [0; HEADER_LEN];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[4], protocol::APPEND | 0x80, "answer kind");
        let mut body = vec![0; u32::from_le_bytes(header[..4].try_into().unwrap()) as usize];
        stream.read_exact(&mut body).unwrap();
        // A caller that has gone has nothing to learn.
        let _ = reply.send(body);
    }
}

/// The committed revision of an `Append` answer's body, or `None` for `Full`.
fn revision(body: &[u8]) -> Option<u64> {
    match u16::from_le_bytes([body[0], body[1]]) {
        // Status, deduplicated, 3 reserved, count, then offset and revision.
        0 => {
            assert_eq!(body[2], 0, "deduplicated: the log was not fresh");
            Some(u64::from_le_bytes(body[18..26].try_into().unwrap()))
        }
        1 => None, // Full (protocol.md)
        status => panic!("append failed with status {status}"),
    }
}

impl Target {
    fn append(&mut self, submit: Submit) -> Option<u64> {
        match self {
            Target::InProcess(log) => match log.append(submit) {
                Ok(pending) => {
                    let committed = pending.wait().expect("commit failure");
                    assert!(
                        !committed.deduplicated,
                        "deduplicated: the log was not fresh"
                    );
                    Some(committed.records[0].revision)
                }
                Err(AppendError::Full) => None,
                Err(error) => panic!("admission failure: {error:?}"),
            },
            Target::Socket(stream) => revision(&exchange(
                stream,
                &Request::Append(submit),
                protocol::APPEND,
            )),
            Target::Shared(connection) => {
                let (reply, answer) = mpsc::channel();
                let request = protocol::encode_request(&Request::Append(submit));
                connection.send((request, reply)).unwrap();
                revision(&answer.recv().unwrap())
            }
        }
    }
}

/// Sends one request and returns its answer's body.
fn exchange(stream: &mut UnixStream, request: &Request, kind: u8) -> Vec<u8> {
    stream
        .write_all(&protocol::encode_request(request))
        .unwrap();
    let mut header = [0; HEADER_LEN];
    stream.read_exact(&mut header).unwrap();
    assert_eq!(header[4], kind | 0x80, "answer kind");
    let mut body = vec![0; u32::from_le_bytes(header[..4].try_into().unwrap()) as usize];
    stream.read_exact(&mut body).unwrap();
    body
}

fn quantile(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    sorted[(sorted.len() * numerator)
        .div_ceil(denominator)
        .saturating_sub(1)]
}

/// A Unix socket, or a Firecracker guest's vsock port through the VMM's socket.
fn dial(mode: &str, path: &str) -> UnixStream {
    if mode != "--firecracker" {
        return UnixStream::connect(path).unwrap();
    }
    let (uds, port) = path.rsplit_once(':').expect("--firecracker <uds>:<port>");
    let mut stream = UnixStream::connect(uds).unwrap();
    stream
        .write_all(format!("CONNECT {port}\n").as_bytes())
        .unwrap();
    // The VMM answers "OK <host port>\n"; read it byte by byte so no protocol byte is consumed.
    let mut reply = Vec::new();
    let mut byte = [0];
    while reply.last() != Some(&b'\n') {
        stream.read_exact(&mut byte).unwrap();
        reply.push(byte[0]);
        assert!(reply.len() <= 32, "no answer to CONNECT");
    }
    assert!(reply.starts_with(b"OK "), "CONNECT refused: {reply:?}");
    stream
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: socket_bench (--inproc <log-dir> | --unix <socket> | --firecracker <uds>:<port>) <callers> <rounds> [<interval-us>] [--connections <n>]";
    let connections = args.iter().position(|a| a == "--connections").map(|at| {
        let n: usize = args.get(at + 1).and_then(|n| n.parse().ok()).expect(usage);
        assert!(n > 0, "{usage}");
        args.drain(at..at + 2);
        n
    });
    let [mode, path, callers, rounds, rest @ ..] = args.as_slice() else {
        panic!("{usage}");
    };
    let callers: usize = callers.parse().expect(usage);
    let rounds: usize = rounds.parse().expect(usage);
    let interval = rest
        .first()
        .map(|us| Duration::from_micros(us.parse().expect(usage)));
    let log = match mode.as_str() {
        "--inproc" => Some(Arc::new(
            SegmentLog::open(path, Options::default()).unwrap(),
        )),
        "--unix" | "--firecracker" => None,
        _ => panic!("{usage}"),
    };
    let options = Options::default();
    println!(
        "CONFIG transport={} callers={callers} connections={} rounds={rounds} payload=256 records_per_submit=1 interval_ns={} linger_ns={} group_byte_cap={} queue_capacity={} pending_byte_budget={}",
        mode.trim_start_matches('-'),
        match (&log, connections) {
            (Some(_), _) => 0,
            (None, n) => n.unwrap_or(callers),
        },
        interval.map_or(0, |i| i.as_nanos()),
        options.linger.as_nanos(),
        options.group_byte_cap,
        options.queue_capacity,
        options.pending_byte_budget,
    );
    // Every connection says Hello before the clock starts.
    let connect = || {
        let mut stream = dial(mode, path);
        let hello = exchange(
            &mut stream,
            &Request::Hello {
                min: protocol::MIN_VERSION,
                max: protocol::MAX_VERSION,
            },
            protocol::HELLO,
        );
        assert_eq!(hello[..2], [0, 0], "hello refused");
        stream
    };
    let shared: Vec<Sender<Exchange>> = match (&log, connections) {
        (None, Some(n)) => (0..n).map(|_| share(connect())).collect(),
        _ => Vec::new(),
    };
    let targets: Vec<Target> = (0..callers)
        .map(|caller| match &log {
            Some(log) => Target::InProcess(Arc::clone(log)),
            None if !shared.is_empty() => Target::Shared(shared[caller % shared.len()].clone()),
            None => Target::Socket(connect()),
        })
        .collect();
    drop(shared);
    let (gates, workers): (Vec<_>, Vec<_>) = targets
        .into_iter()
        .enumerate()
        .map(|(caller, mut target)| {
            let (gate, start) = mpsc::channel::<Instant>();
            let worker = thread::spawn(move || {
                let epoch = start.recv().unwrap();
                let (mut latencies, mut full) = (Vec::with_capacity(rounds), 0u64);
                for revision in 0..rounds {
                    let sent = match interval {
                        None => Instant::now(),
                        Some(interval) => {
                            let due = epoch
                                + interval.mul_f64(caller as f64 / callers as f64)
                                + interval * revision as u32;
                            thread::sleep(due.saturating_duration_since(Instant::now()));
                            due
                        }
                    };
                    let committed = loop {
                        match target.append(input(caller, revision)) {
                            Some(committed) => break committed,
                            None => {
                                full += 1;
                                thread::yield_now();
                            }
                        }
                    };
                    latencies.push(sent.elapsed().as_nanos() as u64);
                    assert_eq!(committed, revision as u64, "revision");
                }
                (latencies, full)
            });
            (gate, worker)
        })
        .unzip();
    let began = Instant::now();
    for gate in gates {
        gate.send(began).unwrap();
    }
    let (mut latencies, mut full) = (Vec::new(), 0);
    for worker in workers {
        let (times, refused) = worker.join().unwrap();
        latencies.extend(times);
        full += refused;
    }
    let elapsed = began.elapsed();
    latencies.sort_unstable();
    let records = callers * rounds;
    println!(
        "APPEND records={records} elapsed_ns={} records_per_second={:.3} full={full}",
        elapsed.as_nanos(),
        records as f64 / elapsed.as_secs_f64()
    );
    println!(
        "METRIC append_ns count={} min={} p50={} p99={} p999={} max={}",
        latencies.len(),
        latencies[0],
        quantile(&latencies, 50, 100),
        quantile(&latencies, 99, 100),
        quantile(&latencies, 999, 1000),
        latencies.last().unwrap()
    );
}
