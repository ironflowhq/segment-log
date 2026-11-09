use super::*;
use crate::acceptance::support::bounded;
fn call(s: &dyn Storage, path: &str, op: Op) -> Value {
    s.call(Call::new(path, Purpose::Open, op)).unwrap()
}
fn setup() -> Sim {
    let s = Sim::new(9);
    call(&s, "/log", Op::CreateDir);
    call(&s, "/", Op::SyncDir);
    call(&s, "/log/a", Op::Create);
    call(&s, "/log", Op::SyncDir);
    s
}
fn read(s: &dyn Storage) -> Vec<u8> {
    match call(s, "/log/a", Op::Read(0, 20000)) {
        Value::Bytes(b) => b,
        v => panic!("expected bytes, got {v:?}"),
    }
}
/// Given a durable file with three unsynced pages
/// When power is lost under a range of seeds
/// Then individual pages can survive with gaps while durable bytes survive
/// Cases: SIM-MODEL
#[test]
fn power_loss_selects_pages_independently() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![1; 12288]));
        call(&s, "/log/a", Op::SyncData);
        call(&s, "/log/a", Op::Write(0, vec![2; 12288]));
        let (seed, trace) = s.trace();
        let mut patterns = std::collections::BTreeSet::new();
        for n in 0..100 {
            let replay = Sim::replay(seed, &trace);
            replay.crash(Crash::PowerLoss, n);
            let b = read(&replay);
            patterns.insert([b[0], b[4096], b[8192]]);
        }
        assert_eq!(patterns.len(), 8);
        assert!(patterns.contains(&[2, 1, 2]));
    });
}
/// Given unsynced writes and directory changes
/// When the process is killed and then power is lost
/// Then kill preserves readability but does not flush the bytes
/// Cases: SIM-MODEL
#[test]
fn kill_keeps_volatile_data_without_making_it_durable() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![7; 4096]));
        s.crash(Crash::Kill, 1);
        assert_eq!(read(&s), vec![7; 4096]);
        let (seed, trace) = s.trace();
        assert!((0..100).any(|n| {
            let r = Sim::replay(seed, &trace);
            r.crash(Crash::PowerLoss, n);
            read(&r).iter().all(|b| *b == 0)
        }));
    });
}
/// Given durable sectors rewritten with identical contents beside changed sectors
/// When a power loss tears a surviving page
/// Then identical sectors survive and changed sectors can tear
/// Cases: SIM-MODEL
#[test]
fn torn_rewrites_preserve_identical_durable_sectors() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![7; 4096]));
        call(&s, "/log/a", Op::SyncAll);
        call(&s, "/log/a", Op::Write(0, vec![7; 4096]));
        call(&s, "/log/a", Op::Write(512, vec![8; 512]));
        let (seed, trace) = s.trace();
        let mut torn = false;
        for n in 0..50 {
            let r = Sim::replay(seed, &trace);
            r.crash(Crash::PowerLossTorn, n);
            let b = read(&r);
            assert_eq!(&b[..512], &[7; 512]);
            assert_eq!(&b[1024..], &[7; 3072]);
            torn |= b[512..1024].iter().any(|x| *x != 7 && *x != 8);
        }
        assert!(torn);
    });
}
/// Given a failed data sync followed by a successful sync without a rewrite
/// When power is lost
/// Then the failed bytes can still be lost, but rewriting restores durability
/// Cases: SIM-MODEL
#[test]
fn failed_sync_requires_rewriting_each_byte() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![3; 4096]));
        let sync = Call::new("/log/a", Purpose::Group, Op::SyncData);
        s.inject(sync.clone(), Fault::error(io::ErrorKind::Other).unwrap());
        assert!(s.call(sync.clone()).is_err());
        assert_eq!(read(&s), vec![3; 4096]);
        s.call(sync.clone()).unwrap();
        let (seed, trace) = s.trace();
        assert!((0..100).any(|n| {
            let r = Sim::replay(seed, &trace);
            r.crash(Crash::PowerLoss, n);
            read(&r).iter().all(|x| *x == 0)
        }));
        call(&s, "/log/a", Op::Write(0, vec![3; 4096]));
        s.call(sync).unwrap();
        for n in 0..20 {
            let (seed, trace) = s.trace();
            let r = Sim::replay(seed, &trace);
            r.crash(Crash::PowerLoss, n);
            assert_eq!(read(&r), vec![3; 4096]);
        }
    });
}
/// Given a new log directory and segment with independently flushed directories
/// When power is lost before either or both directory syncs
/// Then parent reachability and segment reachability are independent
/// Cases: SIM-MODEL
#[test]
fn parent_and_log_entries_need_separate_syncs() {
    bounded(|| {
        for parent_sync in [false, true] {
            for log_sync in [false, true] {
                let s = Sim::new(3);
                call(&s, "/log", Op::CreateDir);
                call(&s, "/log/a", Op::Create);
                if parent_sync {
                    call(&s, "/", Op::SyncDir);
                }
                if log_sync {
                    call(&s, "/log", Op::SyncDir);
                }
                let (seed, trace) = s.trace();
                let mut missing = false;
                for n in 0..100 {
                    let r = Sim::replay(seed, &trace);
                    r.crash(Crash::PowerLoss, n);
                    let exists = r.call(Call::new("/log/a", Purpose::Open, Op::Open)).is_ok();
                    if parent_sync && log_sync {
                        assert!(exists);
                    }
                    missing |= !exists;
                }
                assert_eq!(missing, !(parent_sync && log_sync));
            }
        }
    });
}
/// Given a durable file removed without syncing its directory
/// When power is lost
/// Then the old entry and bytes can return
/// Cases: SIM-MODEL
#[test]
fn unsynced_removal_can_restore_the_durable_file() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![1, 2]));
        call(&s, "/log/a", Op::SyncAll);
        call(&s, "/log/a", Op::Remove);
        let (seed, trace) = s.trace();
        assert!((0..100).any(|n| {
            let r = Sim::replay(seed, &trace);
            r.crash(Crash::PowerLoss, n);
            r.call(Call::new("/log/a", Purpose::Open, Op::Read(0, 2)))
                .ok()
                == Some(Value::Bytes(vec![1, 2]))
        }));
    });
}
/// Given chosen read, write and sync fault boundaries
/// When each fault is injected once
/// Then partial bytes, disk full, corruption, EOF, errors and panic are observable and traced
/// Cases: SIM-MODEL
#[test]
fn injected_faults_have_observable_results_and_replay() {
    bounded(|| {
        let s = setup();
        let write = Call::new("/log/a", Purpose::Group, Op::Append(vec![1, 2, 3]));
        s.inject(write.clone(), Fault::Partial(2));
        assert!(s.call(write.clone()).is_err());
        assert_eq!(read(&s), vec![1, 2]);
        s.inject(write.clone(), Fault::NoSpace);
        assert_eq!(
            s.call(write).unwrap_err().kind(),
            io::ErrorKind::StorageFull
        );
        assert_eq!(read(&s), vec![1, 2]);
        let read_call = Call::new("/log/a", Purpose::Lookup, Op::Read(0, 2));
        for (fault, expected) in [
            (Fault::Corrupt(1, 255), Value::Bytes(vec![1, 253])),
            (Fault::Eof, Value::Bytes(vec![])),
        ] {
            s.inject(read_call.clone(), fault);
            assert_eq!(s.call(read_call.clone()).unwrap(), expected);
        }
        s.inject(
            read_call.clone(),
            Fault::error(io::ErrorKind::Other).unwrap(),
        );
        assert!(s.call(read_call.clone()).is_err());
        s.inject(read_call.clone(), Fault::Panic);
        assert!(std::panic::catch_unwind(|| s.call(read_call)).is_err());
        for op in [Op::SyncAll, Op::SyncData, Op::SyncDir] {
            let path = if op == Op::SyncDir { "/log" } else { "/log/a" };
            let c = Call::new(path, Purpose::Recovery, op);
            s.inject(c.clone(), Fault::error(io::ErrorKind::Other).unwrap());
            assert!(s.call(c).is_err());
        }
        let (seed, trace) = s.trace();
        let path = std::env::temp_dir().join(format!(
            "segment-log-fault-trace-{}.bin",
            std::process::id()
        ));
        crate::acceptance::support::trace::save(&path, seed, &trace).unwrap();
        let (seed, trace) = crate::acceptance::support::trace::load(&path).unwrap();
        assert_eq!(Sim::replay(seed, &trace).image(), s.image());
        std::fs::remove_file(path).unwrap();
    });
}
/// Given a chosen group sync held at entry
/// When another thread reaches it and the scenario releases an error
/// Then the call waits without blocking observations and is counted once by purpose
/// Cases: SIM-MODEL
#[test]
fn held_calls_report_start_and_wait_for_release() {
    bounded(|| {
        let s = setup();
        let c = Call::new("/log/a", Purpose::Group, Op::SyncData);
        let hold = s.hold(c.clone());
        let worker = s.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            tx.send(worker.call(c)).unwrap();
        });
        hold.wait();
        assert!(matches!(
            rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert_eq!(
            s.count(Path::new("/log/a"), Purpose::Group, &Op::SyncData),
            0
        );
        hold.release(Some(Fault::error(io::ErrorKind::Other).unwrap()));
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .is_err()
        );
        thread.join().unwrap();
        assert_eq!(
            s.count(Path::new("/log/a"), Purpose::Group, &Op::SyncData),
            1
        );
        assert_eq!(
            s.count(Path::new("/log/a"), Purpose::Recovery, &Op::SyncData),
            0
        );
    });
}
/// Given identical seeded storage traces
/// When the same crash seed is replayed including chained crashes
/// Then the complete crash states are identical
/// Cases: SIM-MODEL
#[test]
fn trace_replay_reproduces_chained_crash_states() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![3; 9000]));
        s.crash(Crash::Kill, 8);
        s.crash(Crash::PowerLossTorn, 15);
        let (seed, trace) = s.trace();
        let path =
            std::env::temp_dir().join(format!("segment-log-trace-{}.bin", std::process::id()));
        crate::acceptance::support::trace::save(&path, seed, &trace).unwrap();
        let (saved_seed, saved_trace) = crate::acceptance::support::trace::load(&path).unwrap();
        assert_eq!((seed, trace.clone()), (saved_seed, saved_trace.clone()));
        assert_eq!(s.image(), Sim::replay(saved_seed, &saved_trace).image());
        std::fs::remove_file(path).unwrap();
        s.crash(Crash::PowerLoss, 7);
        let r = Sim::replay(seed, &trace);
        r.crash(Crash::PowerLoss, 7);
        assert_eq!(s.image(), r.image());
    });
}
/// Given a lock held across a simulated crash and reacquired in a new boot
/// When the old guard is dropped
/// Then the new lock stays exclusive
/// Cases: SIM-MODEL
#[test]
fn stale_lock_guards_cannot_release_a_new_boot_lock() {
    bounded(|| {
        let s = setup();
        let old = s.lock(Path::new("/log/a")).unwrap();
        s.crash(Crash::Kill, 0);
        let new = s.lock(Path::new("/log/a")).unwrap();
        drop(old);
        assert_eq!(
            s.lock(Path::new("/log/a")).err().unwrap().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(new);
    });
}
/// Given a directory sync error under the storage metadata contract
/// When later writes and directory syncs are attempted
/// Then they fail until reboot while reads remain possible
/// Cases: SIM-MODEL
#[test]
fn metadata_sync_errors_stop_further_writes() {
    bounded(|| {
        let s = setup();
        let sync = Call::new("/log", Purpose::Open, Op::SyncDir);
        s.inject(sync.clone(), Fault::error(io::ErrorKind::Other).unwrap());
        assert!(s.call(sync.clone()).is_err());
        assert!(s.call(sync).is_err());
        assert!(
            s.call(Call::new("/log/a", Purpose::Group, Op::Append(vec![1])))
                .is_err()
        );
        assert_eq!(read(&s), Vec::<u8>::new());
    });
}
/// Given a failed flush of two pages with only one page subsequently rewritten
/// When a later sync succeeds and power is lost
/// Then only the rewritten page is guaranteed durable
/// Cases: SIM-MODEL
#[test]
fn rewriting_one_page_does_not_flush_failed_bytes_in_another() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![3; 8192]));
        let sync = Call::new("/log/a", Purpose::Group, Op::SyncData);
        s.inject(sync.clone(), Fault::error(io::ErrorKind::Other).unwrap());
        assert!(s.call(sync).is_err());
        call(&s, "/log/a", Op::Write(0, vec![4; 4096]));
        call(&s, "/log/a", Op::SyncAll);
        let (seed, trace) = s.trace();
        let mut lost = false;
        for n in 0..100 {
            let r = Sim::replay(seed, &trace);
            r.crash(Crash::PowerLoss, n);
            let b = read(&r);
            assert_eq!(&b[..4096], &[4; 4096]);
            lost |= b[4096..].iter().all(|x| *x == 0);
        }
        assert!(lost);
    });
}
/// Given durable file contents and a shorter truncation
/// When the truncation is synced and the storage crashes
/// Then the durable length and bytes survive each crash mode
/// Cases: SIM-MODEL
#[test]
fn successful_sync_preserves_truncated_length_and_bytes() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![1, 2, 3, 4]));
        call(&s, "/log/a", Op::SyncAll);
        call(&s, "/log/a", Op::Truncate(2));
        call(&s, "/log/a", Op::SyncAll);
        let (seed, trace) = s.trace();
        for mode in [Crash::Kill, Crash::PowerLoss, Crash::PowerLossTorn] {
            for n in 0..20 {
                let r = Sim::replay(seed, &trace);
                r.crash(mode, n);
                assert_eq!(call(&r, "/log/a", Op::Open), Value::Length(2));
                assert_eq!(read(&r), vec![1, 2]);
            }
        }
    });
}
/// Given a chosen write and lookup read held at their starts
/// When the scenario releases each with success
/// Then writes take effect only after release and reads return the written bytes
/// Cases: SIM-MODEL
#[test]
fn writes_and_reads_can_be_held_and_released_successfully() {
    bounded(|| {
        let s = setup();
        for op in [Op::Append(vec![8, 9]), Op::Read(0, 2)] {
            let c = Call::new("/log/a", Purpose::Lookup, op.clone());
            let hold = s.hold(c.clone());
            let worker = s.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                tx.send(worker.call(c)).unwrap();
            });
            hold.wait();
            assert!(matches!(
                rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            if matches!(op, Op::Append(_)) {
                assert_eq!(read(&s), vec![]);
            }
            hold.release(None);
            let expected = if matches!(op, Op::Read(..)) {
                Value::Bytes(vec![8, 9])
            } else {
                Value::Unit
            };
            assert_eq!(
                rx.recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
                    .unwrap(),
                expected
            );
            thread.join().unwrap();
        }
    });
}
/// Given unsynced data whose flush reports disk full
/// When a later flush succeeds without a rewrite and power is lost
/// Then the failed bytes can still be lost
/// Cases: SIM-MODEL
#[test]
fn disk_full_sync_errors_also_require_a_rewrite() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![6; 4096]));
        let c = Call::new("/log/a", Purpose::Group, Op::SyncData);
        s.inject(c.clone(), Fault::NoSpace);
        assert_eq!(
            s.call(c.clone()).unwrap_err().kind(),
            io::ErrorKind::StorageFull
        );
        s.call(c).unwrap();
        let (seed, trace) = s.trace();
        assert!((0..100).any(|n| {
            let r = Sim::replay(seed, &trace);
            r.crash(Crash::PowerLoss, n);
            read(&r).iter().all(|x| *x == 0)
        }));
    });
}

/// Given a durable file removed without syncing its directory
/// When two power losses choose directory entries independently
/// Then a restored file still has its durable contents and storage never panics
/// Cases: SIM-MODEL
#[test]
fn repeated_power_loss_never_restores_an_entry_without_its_file() {
    bounded(|| {
        for first in 0..16 {
            for second in 0..16 {
                let s = setup();
                call(&s, "/log/a", Op::Append(vec![9]));
                call(&s, "/log/a", Op::SyncData);
                call(&s, "/log/a", Op::Remove);
                s.crash(Crash::PowerLoss, first);
                s.crash(Crash::PowerLoss, second);
                let result = std::panic::catch_unwind(|| {
                    s.call(Call::new("/log/a", Purpose::Open, Op::Read(0, 1)))
                });
                assert!(
                    result.is_ok(),
                    "dangling entry after seeds {first}, {second}"
                );
                match result.unwrap() {
                    Ok(bytes) => assert_eq!(bytes, Value::Bytes(vec![9])),
                    Err(error) => assert_eq!(error.kind(), io::ErrorKind::NotFound),
                }
            }
        }
    });
}

/// Given an append held before it changes storage
/// When the simulated process dies and the old call is released
/// Then that call fails and its bytes do not enter the new boot
/// Cases: SIM-MODEL
#[test]
fn a_held_call_cannot_write_after_its_process_dies() {
    bounded(|| {
        let s = setup();
        let c = Call::new("/log/a", Purpose::Group, Op::Append(vec![9]));
        let hold = s.hold(c.clone());
        let worker = s.clone();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || tx.send(worker.call(c)).unwrap());
        hold.wait();
        s.crash(Crash::Kill, 0);
        hold.release(None);
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().is_err());
        assert_eq!(read(&s), Vec::<u8>::new());
        thread.join().unwrap();
        let (seed, trace) = s.trace();
        assert_eq!(Sim::replay(seed, &trace).image(), s.image());
    });
}

/// Given unsynced data whose full-file flush fails
/// When a later full-file flush succeeds without a rewrite
/// Then a power loss can still lose the failed bytes
/// Cases: SIM-MODEL
#[test]
fn failed_full_file_sync_requires_a_rewrite() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![6; 4096]));
        let c = Call::new("/log/a", Purpose::Recovery, Op::SyncAll);
        s.inject(c.clone(), Fault::error(io::ErrorKind::Other).unwrap());
        assert!(s.call(c.clone()).is_err());
        s.call(c).unwrap();
        let (seed, trace) = s.trace();
        assert!((0..100).any(|n| {
            let r = Sim::replay(seed, &trace);
            r.crash(Crash::PowerLoss, n);
            read(&r).iter().all(|x| *x == 0)
        }));
    });
}

/// Given a flushed prefix and an unflushed append sharing one device sector
/// When power loss tears the new suffix
/// Then every previously flushed byte remains unchanged
/// Cases: SIM-MODEL
#[test]
fn tearing_an_append_preserves_the_durable_prefix_in_its_sector() {
    bounded(|| {
        let s = setup();
        call(&s, "/log/a", Op::Append(vec![7; 256]));
        call(&s, "/log/a", Op::SyncData);
        call(&s, "/log/a", Op::Append(vec![8; 256]));
        let (seed, trace) = s.trace();
        for n in 0..100 {
            let r = Sim::replay(seed, &trace);
            r.crash(Crash::PowerLossTorn, n);
            assert_eq!(&read(&r)[..256], &[7; 256], "crash seed {n}");
        }
    });
}

/// Given a sync held in an old process and new writes after a crash
/// When that old sync is released
/// Then cancellation changes no bytes, sync eligibility or directory state
/// Cases: SIM-MODEL
#[test]
fn canceled_syncs_cannot_change_the_new_boot() {
    bounded(|| {
        for op in [Op::SyncData, Op::SyncAll, Op::SyncDir] {
            let s = setup();
            let path = if op == Op::SyncDir { "/log" } else { "/log/a" };
            let c = Call::new(path, Purpose::Group, op);
            let hold = s.hold(c.clone());
            let worker = s.clone();
            let (tx, rx) = mpsc::channel();
            let thread = std::thread::spawn(move || tx.send(worker.call(c)).unwrap());
            hold.wait();
            s.crash(Crash::Kill, 0);
            call(&s, "/log/a", Op::Append(vec![9]));
            let before = s.image();
            hold.release(None);
            assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().is_err());
            thread.join().unwrap();
            assert_eq!(
                s.image(),
                before,
                "cancellation must have no storage effect"
            );
            let (seed, trace) = s.trace();
            let file = std::env::temp_dir()
                .join(format!("segment-log-canceled-sync-{}", std::process::id()));
            crate::acceptance::support::trace::save(&file, seed, &trace).unwrap();
            let (seed, saved) = crate::acceptance::support::trace::load(&file).unwrap();
            assert_eq!(Sim::replay(seed, &saved).image(), before);
            std::fs::remove_file(file).unwrap();
            call(&s, "/log/a", Op::SyncData);
            let (seed, trace) = s.trace();
            for n in 0..16 {
                let replay = Sim::replay(seed, &trace);
                replay.crash(Crash::PowerLoss, n);
                assert_eq!(read(&replay), vec![9]);
            }
        }
    });
}

/// Given an OS error kind without a portable trace representation
/// When a scenario constructs a fault for injection or held-call release
/// Then fault construction rejects it before storage or trace state changes
/// Cases: SIM-MODEL
#[test]
fn unsupported_fault_kinds_are_rejected_before_injection() {
    bounded(|| {
        let kind = io::Error::from_raw_os_error(123456).kind();
        assert!(
            Fault::error(kind).is_err(),
            "unsupported kinds must not enter a trace"
        );
        for kind in ERROR_KINDS {
            assert!(Fault::error(*kind).is_ok());
        }
    });
}
