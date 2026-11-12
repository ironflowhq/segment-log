use super::*;
use crate::storage::{Lock, Storage, Value};
use std::{
    io,
    sync::{Barrier, Mutex},
};

struct ReadOnlyVolume {
    sim: Sim,
    refused: Mutex<Vec<String>>,
}

impl ReadOnlyVolume {
    fn new(sim: &Sim) -> Arc<Self> {
        Arc::new(Self {
            sim: sim.clone(),
            refused: Mutex::new(vec![]),
        })
    }

    fn refused(&self) -> Vec<String> {
        self.refused.lock().unwrap().clone()
    }
}

impl Storage for ReadOnlyVolume {
    fn call(&self, call: Call) -> io::Result<Value> {
        if matches!(call.op, Op::Open | Op::Read(..) | Op::List) {
            return self.sim.call(call);
        }
        self.refused.lock().unwrap().push(format!("{call:?}"));
        Err(io::ErrorKind::ReadOnlyFilesystem.into())
    }

    fn lock(&self, path: &Path) -> io::Result<Box<dyn Lock>> {
        self.refused.lock().unwrap().push(format!("lock {path:?}"));
        Err(io::ErrorKind::ReadOnlyFilesystem.into())
    }
}

fn reader(volume: &Arc<ReadOnlyVolume>) -> Result<LogReader, OpenError> {
    LogReader::with_storage(Path::new("/log"), volume.clone())
}

fn key_index_reads(sim: &Sim) -> usize {
    sim.count(Path::new(ANY_SEG), Purpose::KeyIndex, &Op::Read(0, 0))
}

#[derive(Debug, PartialEq)]
struct Seen {
    log: Observed,
    reads: Vec<String>,
}

fn seen(
    watermark: u64,
    read: impl Fn(u64, usize) -> Vec<Record>,
    head: impl Fn(&[u8]) -> Option<StreamHead>,
) -> Seen {
    let reads = (0..=watermark)
        .map(|from| format!("{:?}", read(from, 3)))
        .collect();
    Seen {
        log: observe(watermark, read(0, usize::MAX), head),
        reads,
    }
}

fn seen_by(reader: &LogReader) -> Seen {
    seen(
        reader.watermark(),
        |from, max| reader.read(from, max).unwrap(),
        |s| reader.stream_head(s),
    )
}

/// Given a log that rolled many times and was sealed, on a volume that refuses every call that
/// could change a file and every lock
/// When two readers open it at once on two threads, and both threads also read through one reader
/// that they share
/// Then each reader gives the watermark, the head of every stream and the records that the sealed
/// log gave, from every offset across segment boundaries
/// And no reader asked the volume for a change or a lock, or read a key filter or a key index
/// And no segment file changed or was removed
/// And `SegmentLog::open` on that volume fails, because it must change files
///
/// Cases: READER, HISTORY, ORACLE-13
#[test]
fn readers_on_a_read_only_volume_read_a_sealed_log() {
    bounded(|| {
        let sim = Sim::new(860);
        let log = open_with(&sim, rolling());
        fill(&log);
        assert!(log.seal().is_ok());
        let expected = seen(
            log.watermark(),
            |from, max| log.read(from, max).unwrap(),
            |s| log.stream_head(s),
        );
        drop(log);
        let segments = raw::segments(&sim);
        assert!(segments.len() > 5, "{} segments", segments.len());
        let (filters, pages) = (key_filter_reads(&sim), key_index_reads(&sim));

        let volume = ReadOnlyVolume::new(&sim);
        let shared = Arc::new(reader(&volume).unwrap());
        let barrier = Arc::new(Barrier::new(2));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let (volume, shared, barrier) = (volume.clone(), shared.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let own = reader(&volume).unwrap();
                    barrier.wait();
                    [seen_by(&own), seen_by(&shared)]
                })
            })
            .collect();
        for thread in threads {
            for view in thread.join().unwrap() {
                assert_eq!(view, expected);
            }
        }
        assert_eq!(volume.refused(), Vec::<String>::new());
        assert_eq!(key_filter_reads(&sim), filters);
        assert_eq!(key_index_reads(&sim), pages);
        assert_eq!(raw::segments(&sim), segments);
        let writer = SegmentLog::with_storage(Path::new("/log"), options(), volume.clone());
        assert!(matches!(writer, Err(OpenError::Io(_))));
    });
}

struct Layout {
    label: &'static str,
    segments: Vec<(u64, Vec<u8>)>,
    /// The records that recovery keeps.
    kept: usize,
}

/// Given a log whose newest segment ends in its groups, in a torn group, in a roll's closing run,
/// or in a seal's run that lost its seal frame, or whose newest file is a header that a crash tore,
/// or a directory with no segment
/// When a reader opens it on a read-only volume
/// Then it fails with `NotSealed`, asks the volume for no change and no lock, and changes no file
/// When `SegmentLog::open` recovers that log on a writable volume and `seal` follows
/// Then a reader on the read-only volume opens it and reads every record that recovery kept
/// And a missing directory fails with `Io` of kind `NotFound`, and a sealed segment with a bad
/// group frame fails with `Corrupt` at that frame
///
/// Cases: READER, RECOVERY-FIXTURES
#[test]
fn a_reader_refuses_a_log_that_is_not_sealed() {
    bounded(|| {
        let groups = [
            raw::header(raw::VERSION, 0, 1),
            raw::group(1, 0, &[b"a"]),
            raw::group(1, 1, &[b"b", b"c"]),
        ]
        .concat();
        let run = raw::seal_run(&groups, 1);
        let torn_group = raw::group(1, 3, &[b"d"]);
        let mut lost_seal = [groups.clone(), run.clone()].concat();
        let seal_len = 8 + crate::codec::BODY_HEADER_LEN;
        let seal_at = lost_seal.len() - crate::codec::LOCATOR_LEN - seal_len;
        lost_seal[seal_at..seal_at + seal_len].fill(0);
        let torn_header = raw::header(raw::VERSION, 3, 1)[..30].to_vec();
        let layout = |label, segments, kept| Layout {
            label,
            segments,
            kept,
        };
        let layouts = [
            layout("groups", vec![(0, groups.clone())], 3),
            layout(
                "torn group",
                vec![(0, [groups.clone(), torn_group[..20].to_vec()].concat())],
                3,
            ),
            layout("roll's run", vec![(0, raw::rolled(groups.clone(), 1))], 3),
            layout("lost seal frame", vec![(0, lost_seal)], 3),
            layout(
                "torn header",
                vec![(0, raw::rolled(groups.clone(), 1)), (3, torn_header)],
                3,
            ),
            layout("no segment", vec![], 0),
        ];
        for Layout {
            label,
            segments,
            kept,
        } in layouts
        {
            let sim = Sim::new(861);
            raw::install_segments(&sim, &segments);
            let before = sim.image();
            let volume = ReadOnlyVolume::new(&sim);
            assert!(
                matches!(reader(&volume), Err(OpenError::NotSealed)),
                "{label}"
            );
            assert_eq!(volume.refused(), Vec::<String>::new(), "{label}");
            assert_eq!(sim.image(), before, "{label}");

            assert!(open(&sim).seal().is_ok(), "{label}");
            let sealed = reader(&volume).unwrap();
            assert_eq!(sealed.read(0, 10).unwrap().len(), kept, "{label}");
            assert_eq!(sealed.watermark(), kept as u64, "{label}");
        }

        let volume = ReadOnlyVolume::new(&Sim::new(861));
        let missing = reader(&volume).err();
        assert!(
            matches!(&missing, Some(OpenError::Io(e)) if e.kind() == io::ErrorKind::NotFound),
            "{missing:?}"
        );

        let mut bad = [groups.clone(), run].concat();
        bad[64 + 40] ^= 1;
        let sim = Sim::new(861);
        raw::install_segments(&sim, &[(0, bad)]);
        let corrupt = reader(&ReadOnlyVolume::new(&sim)).err();
        assert!(
            matches!(
                corrupt,
                Some(OpenError::Corrupt {
                    segment: 0,
                    position: 64
                })
            ),
            "{corrupt:?}"
        );
    });
}
