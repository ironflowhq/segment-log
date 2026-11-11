use super::*;

const SHARED: u64 = 0x5eed_0000_0000_5eed;

fn sharing(keys: &[Vec<u8>]) -> Hooks {
    Hooks {
        key_hashes: keys.iter().map(|key| (key.clone(), SHARED)).collect(),
        ..Hooks::default()
    }
}

/// The hash of each key index entry of a sealed segment image, page by page.
fn index_hashes(segment: &[u8]) -> Vec<Vec<u64>> {
    let (_, run) = raw::split_run(segment);
    let filter = u32::from_le_bytes(run[..4].try_into().unwrap()) as usize + 8;
    let index = &run[filter..];
    let body = u32::from_le_bytes(index[..4].try_into().unwrap()) as usize;
    assert_eq!(index[8], 6, "a key index frame follows the key filter");
    index[32..8 + body]
        .chunks(4096)
        .map(|page| {
            let count = u32::from_le_bytes(page[4080..4084].try_into().unwrap()) as usize;
            page[..16 * count]
                .chunks(16)
                .map(|entry| u64::from_le_bytes(entry[..8].try_into().unwrap()))
                .collect()
        })
        .collect()
}

fn new_record(log: &SegmentLog, stream: &[u8], key: &[u8], payload: &[u8]) -> NewRecord {
    NewRecord {
        stream: stream.to_vec(),
        key: Some(key.to_vec()),
        payload: payload.to_vec(),
        expected: log
            .stream_head(stream)
            .map_or(ExpectedRevision::Empty, |head| {
                ExpectedRevision::At(head.revision)
            }),
    }
}

/// A submit of one record is appended at the watermark, as a new record.
fn assert_appended(log: &SegmentLog, record: NewRecord) {
    let watermark = log.watermark();
    let answer = log
        .append(Submit {
            records: vec![record],
        })
        .unwrap()
        .wait();
    let answer = answer.unwrap_or_else(|error| panic!("a colliding new key got {error:?}"));
    assert!(!answer.deduplicated);
    assert_eq!(offsets(&answer), [watermark]);
}

/// Given keys `a` and `b` that share one hash in one sealed segment, and key `c` with the same hash
/// in a later sealed segment
/// When a new key with that hash arrives, once with the stream and payload of `a`, and once with
/// its own
/// Then each is appended as a new record: never a match, never `KeyConflict`, never a lookup failure
/// And a retry of `a`, `b` or `c`, alone or regrouped, still returns its original position
/// And all of this holds again after a reopen
///
/// Cases: KEY-COLLISION
#[test]
fn a_new_key_that_shares_a_sealed_keys_hash_is_appended() {
    bounded(|| {
        let fresh: Vec<Vec<u8>> = (0..4).map(|n| format!("new-{n}").into_bytes()).collect();
        let hooks = sharing(
            &[
                [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()].to_vec(),
                fresh.clone(),
            ]
            .concat(),
        );
        let sim = Sim::new(350);
        let mut log = open_hooked(&sim, &hooks);
        commit(&log, vec![keyed(b"a"), keyed(b"b")]);
        commit(&log, vec![keyed(b"c")]);
        commit(&log, vec![keyed(b"x")]);
        let segments = raw::segments(&sim);
        assert_eq!(index_hashes(&segments[0].1), [vec![SHARED, SHARED]]);
        assert_eq!(index_hashes(&segments[1].1), [vec![SHARED]]);
        let a = keyed(b"a");
        for (round, reopened) in [false, true].into_iter().enumerate() {
            if reopened {
                drop(log);
                log = open_hooked(&sim, &hooks);
            }
            let same_as_a = new_record(&log, &a.stream, &fresh[2 * round], &a.payload);
            assert_appended(&log, same_as_a);
            let own = new_record(&log, b"own", &fresh[2 * round + 1], b"own payload");
            assert_appended(&log, own);
            assert_retry(&log, &[b"a"], &[0]);
            assert_retry(&log, &[b"b"], &[1]);
            assert_retry(&log, &[b"c", b"a", b"b"], &[2, 0, 1]);
        }
        let retry = commit(
            &log,
            vec![
                new_record(&log, &a.stream, &fresh[0], &a.payload),
                new_record(&log, b"own", &fresh[3], b"own payload"),
            ],
        );
        assert!(retry.deduplicated);
        assert_eq!(offsets(&retry), [4, 7]);
    });
}

/// Given 300 keys that share one hash in one sealed segment, so that their key index entries fill
/// one page and continue on the next
/// When a caller retries the keys on both sides of that page boundary, the first and the last
/// Then each retry returns its original position
/// And a new key with that hash, checked against every one of those entries, is appended
/// And all of this holds again after a reopen
///
/// Cases: KEY-COLLISION
#[test]
fn colliding_keys_on_both_sides_of_a_key_index_page_boundary_still_match() {
    bounded(|| {
        let keys: Vec<Vec<u8>> = (0..300)
            .map(|n| format!("key-{n:03}").into_bytes())
            .collect();
        let fresh: Vec<Vec<u8>> = (0..2).map(|n| format!("new-{n}").into_bytes()).collect();
        let hooks = sharing(&[keys.clone(), fresh.clone()].concat());
        let sim = Sim::new(351);
        let mut log = open_hooked(&sim, &hooks);
        commit(&log, keys.iter().map(|key| keyed(key)).collect());
        commit(&log, vec![keyed(b"x")]);
        let pages = index_hashes(&raw::segments(&sim)[0].1);
        assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), [255, 45]);
        assert!(pages.iter().flatten().all(|&hash| hash == SHARED));
        for (round, reopened) in [false, true].into_iter().enumerate() {
            if reopened {
                drop(log);
                log = open_hooked(&sim, &hooks);
            }
            for n in [254, 255, 0, 299] {
                assert_retry(&log, &[&keys[n]], &[n as u64]);
            }
            assert_retry(&log, &[&keys[255], &keys[254]], &[255, 254]);
            assert_appended(&log, keyed(&fresh[round]));
        }
    });
}
