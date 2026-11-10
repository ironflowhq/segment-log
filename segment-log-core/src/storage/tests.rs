use super::real::Real;
use super::sim::Sim;
use super::*;
use crate::acceptance::support::bounded;
fn call(s: &dyn Storage, path: &str, op: Op) -> Value {
    s.call(Call::new(path, Purpose::Open, op)).unwrap()
}
fn sequence(s: &dyn Storage, root: &str) -> Vec<Value> {
    let file = format!("{root}/a");
    let lock = format!("{root}/LOCK");
    let mut out = vec![];
    for (path, op) in [
        (root, Op::CreateDir),
        (root, Op::SyncDir),
        (&file, Op::Create),
        (&file, Op::Open),
        (&file, Op::Append(vec![1, 2, 3])),
        (&file, Op::Write(1, vec![4, 5, 6])),
        (&file, Op::Read(0, 20)),
        (&file, Op::Open),
        (&file, Op::SyncData),
        (&file, Op::Truncate(2)),
        (&file, Op::SyncAll),
        (&file, Op::Read(0, 20)),
        (&file, Op::Truncate(5)),
        (&file, Op::Read(0, 20)),
        (&lock, Op::Create),
        (root, Op::List),
    ] {
        out.push(call(s, path, op));
    }
    let random = call(s, root, Op::Random(16));
    let Value::Bytes(random) = random else {
        panic!("random bytes missing")
    };
    assert_eq!(random.len(), 16);
    out.push(Value::Length(random.len() as u64));
    assert_eq!(
        s.call(Call::new(&file, Purpose::Open, Op::Create))
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    let first = s.lock(Path::new(&lock)).unwrap();
    assert_eq!(
        s.lock(Path::new(&lock)).err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(first);
    drop(s.lock(Path::new(&lock)).unwrap());
    out.push(call(s, &file, Op::Remove));
    out.push(call(s, root, Op::List));
    assert_eq!(
        s.call(Call::new(&file, Purpose::Open, Op::Open))
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    out
}
/// Given fresh real and simulated directories
/// When identical namespace, byte, length, sync and same-process lock operations run
/// Then all observations match, including lock exclusion and release
/// Cases: SIM-CONFORMANCE
#[test]
fn real_and_simulated_operations_match_without_crashes() {
    bounded(|| {
        let root = std::env::temp_dir().join(format!("segment-log-slice0-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let real = sequence(&Real::default(), root.to_str().unwrap());
        let storage = Sim::new(1);
        let sim = sequence(&storage, "/log");
        let (seed, trace) = storage.trace();
        let path = root.with_extension("trace");
        crate::acceptance::support::trace::save(&path, seed, &trace).unwrap();
        let (seed, trace) = crate::acceptance::support::trace::load(&path).unwrap();
        assert_eq!(storage.image(), Sim::replay(seed, &trace).image());
        std::fs::remove_file(path).unwrap();
        assert_eq!(real, sim);
        std::fs::remove_dir_all(root).unwrap();
    });
}

fn replacement_lock_sequence(s: &dyn Storage, root: &str) {
    call(s, root, Op::CreateDir);
    let path = format!("{root}/LOCK");
    call(s, &path, Op::Create);
    let old = s.lock(Path::new(&path)).unwrap();
    call(s, &path, Op::Remove);
    call(s, &path, Op::Create);
    let new = s.lock(Path::new(&path));
    assert!(
        new.is_ok(),
        "a replacement inode must have an independent lock"
    );
    let new = new.unwrap();
    drop(old);
    assert_eq!(
        s.lock(Path::new(&path)).err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(new);
    drop(s.lock(Path::new(&path)).unwrap());
}

/// Given a locked file that is unlinked and recreated at the same path
/// When the replacement is locked and the old guard is dropped
/// Then both storage implementations lock each inode independently
/// Cases: SIM-CONFORMANCE, SIM-MODEL
#[test]
fn replacing_a_locked_file_does_not_reuse_its_lock() {
    bounded(|| {
        let root = std::env::temp_dir().join(format!(
            "segment-log-lock-replacement-{}",
            std::process::id()
        ));
        replacement_lock_sequence(&Real::default(), root.to_str().unwrap());
        std::fs::remove_dir_all(root).unwrap();
        let sim = Sim::new(1);
        replacement_lock_sequence(&sim, "/log");
        let (seed, trace) = sim.trace();
        assert_eq!(sim.image(), Sim::replay(seed, &trace).image());
    });
}

/// Given a real file whose every eight bytes hold their own position
/// When four threads read chosen positions while another thread appends and flushes
/// Then every read returns the bytes at its position, and the appended bytes stay in order
#[test]
fn real_reads_keep_their_positions_while_another_thread_appends() {
    bounded(|| {
        let root = std::env::temp_dir().join(format!(
            "segment-log-positioned-reads-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let file = root.join("a");
        let file = file.to_str().unwrap();
        let block = |from: u64, words: u64| -> Vec<u8> {
            (from..from + words)
                .flat_map(|word| (word * 8).to_le_bytes())
                .collect()
        };
        let real = Real::default();
        call(&real, root.to_str().unwrap(), Op::CreateDir);
        call(&real, file, Op::Create);
        const SEEDED: u64 = 4096;
        const ADDED: u64 = 256;
        call(&real, file, Op::Append(block(0, SEEDED)));
        std::thread::scope(|scope| {
            for reader in 0..4u64 {
                let real = &real;
                let block = &block;
                scope.spawn(move || {
                    for round in 0..4000u64 {
                        let word = (round * 2654435761 + reader * 977) % (SEEDED - 8);
                        let Value::Bytes(bytes) = call(real, file, Op::Read(word * 8, 64)) else {
                            panic!("bytes missing")
                        };
                        assert_eq!(bytes, block(word, 8), "reader {reader} word {word}");
                    }
                });
            }
            for round in 0..ADDED {
                call(&real, file, Op::Append(block(SEEDED + round, 1)));
                if round % 64 == 0 {
                    call(&real, file, Op::SyncData);
                }
            }
        });
        let all = ((SEEDED + ADDED) * 8) as usize;
        let Value::Bytes(bytes) = call(&real, file, Op::Read(0, all + 8)) else {
            panic!("bytes missing")
        };
        assert_eq!(bytes.len(), all);
        assert!(bytes == block(0, SEEDED + ADDED));
        std::fs::remove_dir_all(root).unwrap();
    });
}
