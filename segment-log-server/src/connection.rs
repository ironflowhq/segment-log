//! One connection: a thread that reads requests and calls the core, and a thread that writes each
//! answer in request order (ADR 0057, ADR 0058).

use crate::log::Log;
use crate::protocol::{self, Admit, Answer, HEADER_LEN, Request, Status};
use crate::server::{EXIT_POISONED, EXIT_STOPPED, Slot};
use segment_log_core::Pending;
use std::io::{self, Read, Write};
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

fn write_answers(mut stream: impl Write, queue: Receiver<Job>, slot: &Slot) {
    for job in queue {
        let bytes = match job {
            Job::Ready(bytes) => bytes,
            Job::Append(pending) => match pending.wait() {
                Ok(committed) => {
                    protocol::encode_answer(protocol::APPEND, &Answer::Append(&committed))
                }
                Err(error) => {
                    let status = Status::from(error);
                    if poisoned(&status) {
                        slot.server.stop(EXIT_POISONED);
                    }
                    failed(protocol::APPEND, status)
                }
            },
        };
        // shortcut: one write per answer; batch answers that are ready together if the socket
        // benchmark shows the syscalls.
        if stream.write_all(&bytes).is_err() {
            return;
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
            Request::Hello { version } if !greeted => {
                if version != protocol::VERSION {
                    let _ = jobs.send(Job::Ready(failed(
                        protocol::HELLO,
                        Status::UnsupportedVersion,
                    )));
                    return;
                }
                greeted = true;
                Job::Ready(protocol::encode_answer(
                    protocol::HELLO,
                    &Answer::Hello { frame_cap },
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
