//! Replays a script on a log over simulated storage and records what the public calls show.
//! This is the only file of the model test that names the log, and it knows nothing of the model.
use super::contract::*;
use crate::acceptance::support::fixtures::{SEG, Sim};
use crate::storage::{Call, Op, Purpose, sim::Hold};
use crate::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

const ANSWER_DEADLINE: Duration = Duration::from_secs(5);

/// Takes `Options` so that a later milestone can rerun the same scripts under other settings.
pub(super) fn drive(script: &Script, options: &Options) -> Trace {
    // Zero linger fixes the members of each group. The margins keep every group far from the queue limit
    // and the byte cap: 64 bytes for each record are more than its entry overhead in format.md.
    // `append` panics below if the byte budget is ever short.
    assert!(options.linger.is_zero());
    assert!(options.queue_capacity >= 64 * GROUP_SUBMITS);
    assert!(options.group_byte_cap >= 64 * (GROUP_BYTES + 64 * GROUP_RECORDS));
    let mut driver = Driver {
        sim: Sim::new(2),
        options,
        streams: script.streams().clone(),
        read_max: script.records() + 1,
        steps: vec![],
        held: None,
        log: None,
    };
    for step in script.steps() {
        driver.step(step);
    }
    driver.finish()
}

enum Slot {
    Known(Outcome),
    Waiting(Pending),
}
/// One step after its flush started: what its callers and the readers saw at that moment.
struct Group {
    slots: Vec<Slot>,
    view: View,
    flushed: usize,
}
impl Group {
    /// `late` resolves the submits that had no answer while the flush was held.
    fn observed(
        self,
        late: impl FnOnce(Vec<(usize, Pending)>) -> Vec<(usize, Answer)>,
    ) -> Observed {
        let count = self.slots.len();
        let mut waiting = vec![];
        let mut outcomes = BTreeMap::new();
        for (index, slot) in self.slots.into_iter().enumerate() {
            match slot {
                Slot::Known(outcome) => {
                    outcomes.insert(index, outcome);
                }
                Slot::Waiting(pending) => waiting.push((index, pending)),
            }
        }
        let late = late(waiting).into_iter();
        outcomes.extend(late.map(|(index, answer)| (index, Outcome::AfterFlush(answer))));
        assert_eq!(outcomes.len(), count, "one outcome for each submit");
        let mut outcomes: Vec<Outcome> = outcomes.into_values().collect();
        let closer = outcomes.pop().expect("every step has a closer");
        Observed {
            outcomes,
            closer,
            held: self.view,
            flushed: self.flushed,
        }
    }
}

struct Driver<'a> {
    sim: Sim,
    options: &'a Options,
    streams: BTreeSet<Bytes>,
    /// One more than the script can write, so that a record too many shows in a read.
    read_max: usize,
    steps: Vec<Observed>,
    /// The newest step. Its flush has started and is held, and the queue is empty.
    held: Option<(Hold, Group)>,
    log: Option<SegmentLog>,
}

impl Driver<'_> {
    fn step(&mut self, step: &Step) {
        if let Step::Open { .. } = step {
            self.release();
            drop(self.log.take());
            let options = Options {
                linger: self.options.linger,
                group_byte_cap: self.options.group_byte_cap,
                roll_bytes: self.options.roll_bytes,
                queue_capacity: self.options.queue_capacity,
                pending_byte_budget: self.options.pending_byte_budget,
            };
            let storage = Arc::new(self.sim.clone());
            self.log = Some(SegmentLog::with_storage(Path::new("/log"), options, storage).unwrap());
        }
        self.group(step.submits(), step.closer());
    }

    fn log(&self) -> &SegmentLog {
        self.log.as_ref().expect("every script starts with an open")
    }

    /// The R12 protocol. The flush before is held while this group enters the queue, so the group's members are fixed.
    /// This group's own flush is held next, so its members are classified and none of its records is durable.
    fn group(&mut self, submits: &[Sub], closer: &Sub) {
        let number = self.steps.len() + usize::from(self.held.is_some());
        let flush = self.sim.hold(Call::new(SEG, Purpose::Group, Op::SyncData));
        let append = |sub: &Sub| match self.log().append(to_submit(sub)) {
            Ok(pending) => Some(pending),
            Err(AppendError::Invalid(_)) => None,
            Err(other) => panic!("step {number}: append returned {other:?} for {sub:?}"),
        };
        let handles: Vec<Option<Pending>> = submits.iter().chain([closer]).map(append).collect();
        let before = self.held.take().map(|(hold, group)| {
            hold.release(None);
            group
        });
        if catch_unwind(AssertUnwindSafe(|| flush.wait())).is_err() {
            let probes: Vec<_> = handles
                .iter()
                .map(|h| h.as_ref().map(Pending::probe))
                .collect();
            panic!(
                "step {number}: the group has a new record, but its flush did not start. Answers by submit: {probes:?}"
            );
        }
        // Write path step 5 comes before the next group's step 2, so every answer of the group before is recorded.
        if let Some(group) = before {
            let probe = |(index, pending): (usize, Pending)| {
                let answer = pending.probe().unwrap_or_else(|| {
                    panic!(
                        "step {}: submit {index} had no answer when the next group's flush started",
                        number - 1
                    )
                });
                (index, to_answer(answer))
            };
            self.steps
                .push(group.observed(|waiting| waiting.into_iter().map(probe).collect()));
        }
        let slot = |handle: Option<Pending>| match handle {
            None => Slot::Known(Outcome::Refused),
            Some(pending) => match pending.probe() {
                Some(answer) => Slot::Known(Outcome::AtClassification(to_answer(answer))),
                None => Slot::Waiting(pending),
            },
        };
        let group = Group {
            slots: handles.into_iter().map(slot).collect(),
            view: self.view(),
            flushed: self.flushed(),
        };
        self.held = Some((flush, group));
    }

    /// Lets the held group finish. No later flush marks the moment its answers exist, so this waits for them.
    fn release(&mut self) {
        let Some((hold, group)) = self.held.take() else {
            return;
        };
        let number = self.steps.len();
        hold.release(None);
        let wait = |waiting: Vec<(usize, Pending)>| {
            let (send, answers) = mpsc::channel();
            let mut missing: BTreeSet<usize> = waiting.iter().map(|(index, _)| *index).collect();
            for (index, pending) in waiting {
                let send = send.clone();
                thread::spawn(move || send.send((index, pending.wait())));
            }
            let deadline = Instant::now() + ANSWER_DEADLINE;
            let mut answered = vec![];
            while !missing.is_empty() {
                let left = deadline.saturating_duration_since(Instant::now());
                let Ok((index, answer)) = answers.recv_timeout(left) else {
                    panic!(
                        "step {number}: submits {missing:?} had no answer {ANSWER_DEADLINE:?} after the group's flush was released"
                    );
                };
                missing.remove(&index);
                answered.push((index, to_answer(answer)));
            }
            answered
        };
        self.steps.push(group.observed(wait));
    }

    fn finish(mut self) -> Trace {
        self.release();
        Trace {
            end: self.view(),
            flushed: self.flushed(),
            steps: std::mem::take(&mut self.steps),
        }
    }

    fn view(&self) -> View {
        let log = self.log();
        let record = |r: Record| Rec {
            pos: Pos {
                offset: r.offset,
                revision: r.revision,
            },
            stream: Bytes(r.stream),
            key: r.key.map(Bytes),
            payload: Bytes(r.payload),
        };
        let head = |stream: &Bytes| {
            let head = log.stream_head(&stream.0);
            (
                stream.clone(),
                head.map(|h| Pos {
                    offset: h.offset,
                    revision: h.revision,
                }),
            )
        };
        View {
            watermark: log.watermark(),
            records: log
                .read(0, self.read_max)
                .unwrap()
                .into_iter()
                .map(record)
                .collect(),
            heads: self.streams.iter().map(head).collect(),
        }
    }

    fn flushed(&self) -> usize {
        self.sim
            .count(Path::new(SEG), Purpose::Group, &Op::SyncData)
    }
}

impl Drop for Driver<'_> {
    fn drop(&mut self) {
        // Dropping the log joins the writer, which is parked in the held flush after a failed step.
        if let Some((hold, _)) = self.held.take() {
            // `release` panics once the writer gave up on the hold.
            // A second panic during an unwind aborts the process, which ends shrinking.
            let _ = catch_unwind(AssertUnwindSafe(|| hold.release(None)));
        }
    }
}

fn to_submit(sub: &Sub) -> Submit {
    let record = |r: &NewRec| NewRecord {
        stream: r.stream.0.clone(),
        key: r.key.as_ref().map(|key| key.0.clone()),
        payload: r.payload.0.clone(),
        expected: match r.expected {
            Expect::Empty => ExpectedRevision::Empty,
            Expect::At(revision) => ExpectedRevision::At(revision),
        },
    };
    Submit {
        records: sub.0.iter().map(record).collect(),
    }
}

/// The one place where the log's answers become harness values. No script here can cause the other errors.
fn to_answer(result: Result<Committed, CommitError>) -> Answer {
    match result {
        Ok(committed) => Ok(Done {
            positions: committed
                .records
                .iter()
                .map(|p| Pos {
                    offset: p.offset,
                    revision: p.revision,
                })
                .collect(),
            deduplicated: committed.deduplicated,
        }),
        Err(CommitError::WrongRevision { index, actual }) => {
            Err(Reject::WrongRevision { index, actual })
        }
        Err(CommitError::KeyConflict { index }) => Err(Reject::KeyConflict { index }),
        Err(CommitError::PartialRetry) => Err(Reject::PartialRetry),
        Err(
            other @ (CommitError::Exhausted | CommitError::Indeterminate | CommitError::Poisoned),
        ) => {
            panic!("the log answered {other:?}, which no script here can cause")
        }
    }
}
