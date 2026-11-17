//! One connection: a thread that reads requests and calls the core, and a thread that writes each
//! answer in request order (ADR 0057, ADR 0058).

use crate::log::Log;
use crate::protocol::{self, Admit, Answer, HEADER_LEN, Request, Status};
use crate::server::{EXIT_POISONED, EXIT_STOPPED, Slot};
use segment_log_core::{CommitError, Committed, Pending};
use std::io::{self, BufReader, Read, Write};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;

/// Requests read but not yet answered on one connection. The reader stops reading when the
/// writer falls this far behind, so a client that never reads its answers stalls itself, and the
/// server's memory for it stays bounded.
const PIPELINE_DEPTH: usize = 64;

enum Job {
    Ready(Vec<u8>),
    Append(Pending),
}

/// Starts both threads. `slot` is released when both have finished with the connection.
///
/// A thread that cannot start fails only this connection: what it would have owned is dropped,
/// which closes the stream and, through the closed queue, ends the other thread.
pub fn serve<S>(stream: S, writer: S, log: Arc<Log>, frame_cap: usize, slot: Slot) -> io::Result<()>
where
    S: Read + Write + Send + 'static,
{
    let (jobs, queue) = sync_channel(PIPELINE_DEPTH);
    let slot = Arc::new(slot);
    let answer_slot = Arc::clone(&slot);
    thread::Builder::new().spawn(move || write_answers(writer, queue, &answer_slot))?;
    // Buffered, so that requests a client wrote together cost one read, not two each.
    let stream = BufReader::new(stream);
    thread::Builder::new().spawn(move || read_requests(stream, &log, frame_cap, &jobs, &slot))?;
    Ok(())
}

/// A poisoned writer cannot recover in place (ADR 0004): the server stops, and a new open recovers.
fn poisoned(status: &Status) -> bool {
    matches!(
        status,
        Status::Poisoned | Status::PoisonedBeforeWrite | Status::Indeterminate
    )
}

/// Stops adding ready answers to one write past this size. Queued answers already bound the
/// connection's memory (`PIPELINE_DEPTH`); this keeps a batch from doubling it.
const BATCH_BYTES: usize = 64 * 1024;

/// Sends every answer that is ready together in one write, in request order: each write costs a
/// packet through the guest's vsock, and the VMM's one muxer thread bounds the packets.
fn write_answers(mut stream: impl Write, queue: Receiver<Job>, slot: &Slot) {
    let mut head = None;
    let mut bytes = Vec::new();
    loop {
        let Some(job) = head.take().or_else(|| queue.recv().ok()) else {
            return;
        };
        bytes.extend(match job {
            Job::Ready(answer) => answer,
            Job::Append(pending) => append_answer(pending.wait(), slot),
        });
        while bytes.len() < BATCH_BYTES {
            match queue.try_recv() {
                Ok(Job::Ready(answer)) => bytes.extend(answer),
                Ok(Job::Append(pending)) => match pending.try_wait() {
                    Ok(result) => bytes.extend(append_answer(result, slot)),
                    Err(pending) => {
                        head = Some(Job::Append(pending));
                        break;
                    }
                },
                Err(_) => break,
            }
        }
        if stream.write_all(&bytes).is_err() {
            return;
        }
        bytes.clear();
    }
}

fn append_answer(result: Result<Committed, CommitError>, slot: &Slot) -> Vec<u8> {
    match result {
        Ok(committed) => protocol::encode_answer(protocol::APPEND, &Answer::Append(&committed)),
        Err(error) => {
            let status = Status::from(error);
            if poisoned(&status) {
                slot.server.stop(EXIT_POISONED);
            }
            failed(protocol::APPEND, status)
        }
    }
}

fn failed(kind: u8, status: Status) -> Vec<u8> {
    protocol::encode_answer(kind, &Answer::Failed(status))
}

/// Returns when the client leaves, after a malformed request, or when the writer has gone.
/// Dropping `jobs` then lets the writer finish the answers already queued.
fn read_requests(
    mut stream: impl Read,
    log: &Log,
    frame_cap: usize,
    jobs: &SyncSender<Job>,
    slot: &Slot,
) {
    let malformed = || {
        let _ = jobs.send(Job::Ready(failed(
            protocol::MALFORMED_KIND,
            Status::Malformed,
        )));
    };
    let mut greeted = false;
    loop {
        let mut header = [0; HEADER_LEN];
        if stream.read_exact(&mut header).is_err() {
            return;
        }
        let Ok(header) = protocol::decode_header(header) else {
            return malformed();
        };
        match protocol::admit(header, frame_cap) {
            Err(_) => return malformed(),
            Ok(Admit::Discard) => {
                let mut body = (&mut stream).take(header.body_len as u64);
                if !greeted
                    || io::copy(&mut body, &mut io::sink()).ok() != Some(header.body_len as u64)
                {
                    return malformed();
                }
                if jobs
                    .send(Job::Ready(failed(protocol::APPEND, Status::TooLarge)))
                    .is_err()
                {
                    return;
                }
                continue;
            }
            Ok(Admit::Read) => {}
        }
        let mut body = vec![0; header.body_len];
        if stream.read_exact(&mut body).is_err() {
            return;
        }
        let Ok(request) = protocol::decode_request(header.kind, &body) else {
            return malformed();
        };
        let job = match request {
            Request::Hello { min, max } if !greeted => {
                let server = (protocol::MIN_VERSION, protocol::MAX_VERSION);
                let Some(version) = protocol::negotiate((min, max), server) else {
                    let _ = jobs.send(Job::Ready(failed(
                        protocol::HELLO,
                        Status::UnsupportedVersion,
                    )));
                    return;
                };
                greeted = true;
                Job::Ready(protocol::encode_answer(
                    protocol::HELLO,
                    &Answer::Hello { version, frame_cap },
                ))
            }
            _ if !greeted => return malformed(),
            Request::Hello { .. } => return malformed(),
            Request::Append(submit) => match log.append(submit) {
                Ok(pending) => Job::Append(pending),
                Err(error) => {
                    let status = Status::from(error);
                    if poisoned(&status) {
                        slot.server.stop(EXIT_POISONED);
                    }
                    Job::Ready(failed(protocol::APPEND, status))
                }
            },
            Request::Read { from, max } => {
                let bytes = match log.read(from, protocol::read_limit(max, frame_cap)) {
                    Ok(records) => {
                        let fit = protocol::fitting_records(&records, frame_cap);
                        protocol::encode_answer(protocol::READ, &Answer::Read(&records[..fit]))
                    }
                    Err(error) => failed(protocol::READ, error.into()),
                };
                Job::Ready(bytes)
            }
            Request::Watermark => Job::Ready(protocol::encode_answer(
                protocol::WATERMARK,
                &Answer::Watermark(log.watermark()),
            )),
            Request::StreamHead(stream) => Job::Ready(protocol::encode_answer(
                protocol::STREAM_HEAD,
                &Answer::StreamHead(log.stream_head(&stream).as_ref()),
            )),
            Request::Seal => Job::Ready(match log.seal() {
                Ok(stop) => {
                    // A sealed log takes no appends, so a writer's work is done (ADR 0058).
                    // The answer still goes out: stopping ends reads, not the queued answers.
                    if stop {
                        slot.server.stop(EXIT_STOPPED);
                    }
                    protocol::encode_answer(protocol::SEAL, &Answer::Seal)
                }
                Err(error) => {
                    slot.server.stop(EXIT_POISONED);
                    failed(protocol::SEAL, error.into())
                }
            }),
        };
        if jobs.send(job).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::Server;
    use segment_log_core::{ExpectedRevision, NewRecord, Options, SegmentLog, Submit};
    use std::io::Cursor;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Both halves of a connection: reads come from a fixed burst, then end; every read and
    /// write call is recorded.
    #[derive(Clone)]
    struct Stream {
        input: Arc<Mutex<Cursor<Vec<u8>>>>,
        reads: Arc<AtomicUsize>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.input.lock().unwrap().read(buf)
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes.lock().unwrap().push(buf.to_vec());
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn append(place: usize) -> Request {
        Request::Append(Submit {
            records: vec![NewRecord {
                stream: format!("s{place}").into_bytes(),
                expected: ExpectedRevision::Empty,
                key: None,
                payload: vec![0x5a; 256],
            }],
        })
    }

    /// Sends Hello and `appends` appends in one burst to a fresh log, and returns the stream once
    /// both of the connection's threads have finished.
    fn burst(appends: usize) -> Stream {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "segment-log-connection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        // A directory left by a killed run of a reused process id.
        let _ = std::fs::remove_dir_all(&dir);
        let options = Options::default();
        let frame_cap = protocol::frame_cap(options.group_byte_cap);
        let log = Arc::new(Log::Writer(SegmentLog::open(&dir, options).unwrap()));
        let mut input = protocol::encode_request(&Request::Hello {
            min: protocol::MIN_VERSION,
            max: protocol::MAX_VERSION,
        });
        (0..appends).for_each(|place| input.extend(protocol::encode_request(&append(place))));
        let stream = Stream {
            input: Arc::new(Mutex::new(Cursor::new(input))),
            reads: Arc::default(),
            writes: Arc::default(),
        };
        let server = Server::new(1);
        let slot = server.admit(Box::new(|| {})).unwrap();
        serve(stream.clone(), stream.clone(), log, frame_cap, slot).unwrap();
        server.wait_until_idle(Duration::from_secs(10));
        std::fs::remove_dir_all(dir).unwrap();
        stream
    }

    /// Splits the written bytes into answers: (kind, body).
    fn answers(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut answers = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            let len = u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize;
            answers.push((rest[4], rest[HEADER_LEN..HEADER_LEN + len].to_vec()));
            rest = &rest[HEADER_LEN + len..];
        }
        answers
    }

    /// Given a fresh log and a client that sends Hello and 200 appends in one burst
    /// When the server answers them
    /// Then every answer arrives once, in request order, at offsets 0 to 199
    /// And answers that are ready together share a write, so there are far fewer writes than answers
    #[test]
    fn ready_answers_share_one_write_in_request_order() {
        let stream = burst(200);
        let writes = stream.writes.lock().unwrap();
        let answers = answers(&writes.concat());
        assert_eq!(answers.len(), 201);
        assert_eq!(answers[0].0, protocol::HELLO | 0x80);
        for (place, (kind, body)) in answers[1..].iter().enumerate() {
            assert_eq!(*kind, protocol::APPEND | 0x80, "answer {place}");
            assert_eq!(u16::from_le_bytes([body[0], body[1]]), 0, "answer {place}");
            // Status, deduplicated, 3 reserved, count, then offset and revision (protocol.md).
            let offset = u64::from_le_bytes(body[10..18].try_into().unwrap());
            assert_eq!(offset, place as u64, "answer {place}");
        }
        // Loose, because group formation depends on timing: without sharing it is 201.
        assert!(
            writes.len() <= 100,
            "{} writes for 201 answers",
            writes.len()
        );
    }

    /// Given a client that sends Hello and 200 appends in one burst
    /// When the server reads them
    /// Then it reads the burst in a few large reads, not two reads per request
    #[test]
    fn a_burst_of_requests_costs_few_reads() {
        let stream = burst(200);
        let reads = stream.reads.load(Ordering::SeqCst);
        // Without a buffer it is two for each request, 402 or more.
        assert!(reads <= 100, "{reads} reads for 201 requests");
    }
}
