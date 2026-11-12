use super::*;
use crate::engine::KeyMemory;

fn filter_bytes(n: usize) -> usize {
    8 * n.div_ceil(255) + 64 * n.div_ceil(32)
}

fn keys(prefix: &str, n: usize) -> Vec<NewRecord> {
    (0..n)
        .map(|i| keyed(format!("{prefix}-{i}").as_bytes()))
        .collect()
}

/// Given sealed segments that hold 300, 0 and 32 keyed records, and an active segment with 5
/// When the key memory is counted, before and after a reopen
/// Then the filters and fences in memory take 656, 0 and 72 bytes, as format.md sizes them
/// And there are 332 sealed keys beside 5 keys in the active segment's map
/// And the log key filter covers the 3 sealed segments with room for twice the keys it was
/// sized for, at 8 bits per key: 300 keys at the first roll, 332 at the reopen (ADR 0051)
///
/// Cases: KEY-MEMORY
#[test]
fn key_memory_is_the_filters_and_fences_of_sealed_keys_beside_the_active_map() {
    bounded(|| {
        assert_eq!(
            [filter_bytes(300), filter_bytes(0), filter_bytes(32)],
            [656, 0, 72]
        );
        let sim = Sim::new(360);
        let hooks = Hooks::default();
        let mut log = open_hooked(&sim, &hooks);
        commit(&log, keys("first", 300));
        commit(
            &log,
            vec![NewRecord {
                stream: b"plain".to_vec(),
                key: None,
                payload: vec![],
                expected: ExpectedRevision::Empty,
            }],
        );
        commit(&log, keys("third", 32));
        commit(&log, keys("active", 5));
        assert_eq!(raw::segments(&sim).len(), 4);
        let log_filter_bytes = |keys: usize| 64 * (2 * keys).div_ceil(64);
        let expected = KeyMemory {
            filter_bytes: 728,
            sealed_keys: 332,
            active_keys: 5,
            log_filter_bytes: log_filter_bytes(300),
            log_filter_covered: 3,
        };
        assert_eq!(log.key_memory(), expected);
        drop(log);
        log = open_hooked(&sim, &hooks);
        assert_eq!(
            log.key_memory(),
            KeyMemory {
                log_filter_bytes: log_filter_bytes(332),
                ..expected
            }
        );
    });
}
