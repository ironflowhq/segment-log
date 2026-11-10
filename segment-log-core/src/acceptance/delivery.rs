use super::support::{bounded, fixtures::*};
use crate::storage::sim::{Crash, Fault};
use crate::storage::{Call, Op, Purpose};
use crate::*;
use std::{path::Path, thread};

/// What a caller does with its result handle while its group is at the flush.
#[derive(Clone, Copy, PartialEq)]
enum Caller {
    /// Parked in `wait` before the group is answered.
    Parked,
    /// Calls `wait` only after the group is answered.
    Late,
    /// Keeps the handle and never waits.
    Idle,
    /// Dropped the handle.
    Gone,
}

#[derive(Clone, Copy)]
enum End {
    Success,
    Failure,
    Crash(Crash),
}

fn request(place: usize) -> Submit {
    Submit {
        records: vec![NewRecord {
            stream: format!("s{place}").into_bytes(),
            expected: ExpectedRevision::Empty,
            key: None,
            payload: (place as u64).to_le_bytes().to_vec(),
        }],
    }
}

fn check(place: usize, end: End, answer: Result<Committed, CommitError>) {
    match end {
        End::Success => {
            let answer = answer.unwrap();
            assert!(!answer.deduplicated);
            assert_eq!(answer.records.len(), 1, "place {place}");
            assert_eq!(
                (answer.records[0].offset, answer.records[0].revision),
                (1 + place as u64, 0),
                "place {place}"
            );
        }
        End::Failure | End::Crash(_) => assert!(
            matches!(answer, Err(CommitError::Indeterminate)),
            "place {place}"
        ),
    }
}

/// Queues `size` submits behind a held flush so that they form one group, lets each caller
/// take its role while that group is at its own flush, ends the flush, and checks every answer.
fn deliver(size: usize, role: impl Fn(usize) -> Caller, end: End) {
    let sim = Sim::new(4400 + size as u64);
    let log = open(&sim);
    let bootstrap_hold = sim.hold(Call::new(SEG, Purpose::Group, Op::SyncData));
    let bootstrap = log.append(request(usize::MAX)).unwrap();
    bootstrap_hold.wait();
    let handles: Vec<_> = (0..size)
        .map(|place| log.append(request(place)).unwrap())
        .collect();
    let hold = sim.hold(Call::new(SEG, Purpose::Group, Op::SyncData));
    bootstrap_hold.release(None);
    assert_eq!(bootstrap.wait().unwrap().records[0].offset, 0);
    hold.wait();
    let mut parked = Vec::new();
    let mut kept = Vec::new();
    for (place, handle) in handles.into_iter().enumerate() {
        match role(place) {
            Caller::Parked => {
                let is_parked = handle.parked();
                let thread = thread::spawn(move || handle.wait());
                // Bounded by the scenario watchdog. The relay needs the caller registered.
                while !is_parked() {
                    thread::yield_now();
                }
                parked.push((place, thread));
            }
            Caller::Gone => drop(handle),
            role => {
                assert!(handle.probe().is_none(), "place {place}");
                kept.push((place, role, handle));
            }
        }
    }
    for (place, thread) in &parked {
        assert!(!thread.is_finished(), "place {place}");
    }
    assert_eq!(log.watermark(), 1);
    match end {
        End::Success => hold.release(None),
        End::Failure => hold.release(Some(Fault::NoSpace)),
        End::Crash(mode) => {
            sim.crash(mode, size as u64);
            hold.release(None);
        }
    }
    for (place, thread) in parked {
        check(place, end, thread.join().unwrap());
    }
    let (seed, trace) = match end {
        End::Success => {
            // The writer takes this submit only after it has finished answering the group.
            let after = log.append(request(size)).unwrap().wait().unwrap();
            assert_eq!(after.records[0].offset, 1 + size as u64);
            assert_eq!(sim.count(Path::new(SEG), Purpose::Group, &Op::SyncData), 3);
            let history = log.read(0, size + 3).unwrap();
            assert_eq!(history.len(), size + 2);
            let trace = sim.trace();
            drop(log);
            trace
        }
        End::Failure | End::Crash(_) => {
            assert!(matches!(
                log.append(request(size)).map(Pending::wait),
                Err(AppendError::Poisoned) | Ok(Err(CommitError::Poisoned))
            ));
            // Drop joins the writer, so every answer exists afterwards.
            drop(log);
            sim.trace()
        }
    };
    for (place, role, handle) in kept {
        let answer = match role {
            Caller::Late => handle.wait(),
            _ => handle
                .probe()
                .expect("an answered group leaves no caller without an answer"),
        };
        check(place, end, answer);
    }
    match end {
        End::Success => {
            for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
                let replay = Sim::replay(seed, &trace);
                replay.crash(mode, size as u64);
                let records = open(&replay).read(0, size + 3).unwrap();
                assert_eq!(records.len(), size + 2);
                for (offset, record) in records.iter().enumerate().skip(1) {
                    assert_eq!(record.offset, offset as u64);
                    assert_eq!(record.payload, (offset as u64 - 1).to_le_bytes());
                }
            }
        }
        End::Failure => {
            sim.crash(Crash::Kill, 0);
            assert_eq!(open(&sim).watermark(), 1 + size as u64);
        }
        End::Crash(mode) => {
            let kept = open(&sim).watermark();
            assert!(kept == 1 || kept == 1 + size as u64, "kept {kept}");
            if matches!(mode, Crash::Kill) {
                assert_eq!(kept, 1 + size as u64);
            }
        }
    }
}

fn mixes() -> [fn(usize) -> Caller; 4] {
    [
        |_| Caller::Parked,
        |place| [Caller::Parked, Caller::Late, Caller::Idle, Caller::Gone][place % 4],
        // The writer's own places are not parked, so it must reach the callers below them.
        |place| {
            if place < 4 {
                Caller::Late
            } else {
                Caller::Parked
            }
        },
        |place| {
            if place % 5 == 4 {
                Caller::Parked
            } else {
                Caller::Gone
            }
        },
    ]
}

/// Given one group at its flush whose callers are parked in `wait`, will wait later,
/// only hold their handle, or dropped it, in several mixes and in groups of 1 to 85
/// When the flush succeeds
/// Then every parked caller wakes with its own position, whatever the callers above it do
/// And a later `wait` returns its own position at once, and an unused handle holds its answer
/// And every acknowledged record survives each crash mode
/// Cases: ACK-DELIVERY, ORACLE-1, GROUP-FORMATION
#[test]
fn every_caller_of_a_durable_group_gets_its_own_answer() {
    bounded(|| {
        for size in [1, 4, 5, 20, 21, 85] {
            for role in mixes() {
                deliver(size, role, End::Success);
            }
        }
    });
}

/// Given one group of 21 at its flush with parked, late, idle and gone callers
/// When the flush fails
/// Then every caller gets `Indeterminate`, parked or not, and the writer is poisoned
/// And the written group is still a whole group after reopen
/// Cases: ACK-DELIVERY, ORACLE-7
#[test]
fn every_caller_of_a_failed_group_gets_an_unknown_outcome() {
    bounded(|| {
        for role in mixes() {
            deliver(21, role, End::Failure);
        }
    });
}

/// Given one group of 21 at its flush with parked, late, idle and gone callers
/// When Kill, PowerLoss or PowerLossTorn interrupts the flush
/// Then no caller gets a success, every caller gets `Indeterminate`
/// And reopen exposes the whole group or none of it, with Kill retaining it
/// Cases: ACK-DELIVERY, ORACLE-2, ORACLE-3
#[test]
fn every_caller_of_a_crashed_group_gets_an_unknown_outcome() {
    bounded(|| {
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            for role in mixes() {
                deliver(21, role, End::Crash(mode));
            }
        }
    });
}
