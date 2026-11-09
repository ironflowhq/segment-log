pub(crate) use crate::storage::sim::Sim;
use crate::*;
use std::{path::Path, sync::Arc, time::Duration};
pub(crate) const SEG: &str = "/log/00000000000000000000.seg";
pub(crate) fn options() -> Options {
    Options {
        linger: Duration::ZERO,
        ..Options::default()
    }
}
pub(crate) fn open(sim: &Sim) -> SegmentLog {
    SegmentLog::with_storage(Path::new("/log"), options(), Arc::new(sim.clone())).unwrap()
}
pub(crate) fn submit(stream: &[u8]) -> Submit {
    Submit {
        records: vec![NewRecord {
            stream: stream.to_vec(),
            expected: ExpectedRevision::Empty,
            key: Some(b"key".to_vec()),
            payload: vec![0, 255, 7],
        }],
    }
}
pub(crate) fn append(log: &SegmentLog, stream: &[u8]) -> Pending {
    let result = log.append(submit(stream));
    assert!(result.is_ok(), "valid append must be admitted");
    result.unwrap_or_else(|_| unreachable!())
}
pub(crate) fn assert_record(log: &SegmentLog) {
    let records = log.read(0, 100).unwrap();
    assert_eq!(records.len(), 1);
    let r = &records[0];
    assert_eq!((r.offset, r.revision), (0, 0));
    assert_eq!(r.stream, b"s");
    assert_eq!(r.key.as_deref(), Some(b"key".as_slice()));
    assert_eq!(r.payload, [0, 255, 7]);
    assert_eq!(log.watermark(), 1);
    let head = log.stream_head(b"s").unwrap();
    assert_eq!((head.offset, head.revision), (0, 0));
}

pub(crate) fn first_frame() -> Vec<u8> {
    crate::codec::encode(&crate::codec::Group {
        epoch: 1,
        base_offset: 0,
        entries: vec![crate::codec::Entry {
            stream: b"s",
            key: Some(b"key"),
            payload: &[0, 255, 7],
            revision: 0,
        }],
    })
    .unwrap()
}

pub(crate) fn epoch_fixture(
    sim: &Sim,
    header_epoch: u64,
    group_epoch: Option<u64>,
    invalid_tail: bool,
) {
    use crate::storage::{Call, Op, Purpose, Storage};
    drop(open(sim));
    let mut bytes = crate::codec::encode_header(&crate::codec::Header {
        log_id: [7; 16],
        base_offset: 0,
        epoch: header_epoch,
    })
    .to_vec();
    if let Some(epoch) = group_epoch {
        bytes.extend(
            crate::codec::encode(&crate::codec::Group {
                epoch,
                base_offset: 0,
                entries: vec![crate::codec::Entry {
                    stream: b"s",
                    key: None,
                    payload: b"",
                    revision: 0,
                }],
            })
            .unwrap(),
        );
    }
    if invalid_tail {
        bytes.extend([0xff; 9]);
    }
    sim.call(Call::new(SEG, Purpose::Recovery, Op::Truncate(0)))
        .unwrap();
    sim.call(Call::new(SEG, Purpose::Recovery, Op::Write(0, bytes)))
        .unwrap();
    sim.call(Call::new(SEG, Purpose::Recovery, Op::SyncAll))
        .unwrap();
}

pub(crate) fn assert_records(actual: &[Record], expected: &[Record]) {
    assert_eq!(actual.len(), expected.len());
    for (a, e) in actual.iter().zip(expected) {
        assert_eq!((a.offset, a.revision), (e.offset, e.revision));
        assert_eq!(
            (&a.stream, &a.key, &a.payload),
            (&e.stream, &e.key, &e.payload)
        );
    }
}
