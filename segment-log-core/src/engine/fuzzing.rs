//! Entry points for the `cargo fuzz` targets in `fuzz/`. They compile only under `cfg(fuzzing)`,
//! so no release build has them. Each one panics only when it finds a bug.
//!
//! The first input byte picks a mode. In checksummed mode the entry point rewrites every frame CRC
//! before decoding, because random bytes almost never pass a CRC, and the fuzzer would otherwise
//! never reach the body checks.

use super::*;
use crate::storage::sim::Sim;

const LOG_ID: [u8; 16] = [7; 16];

/// `FUZZ-DECODER`: the header decoder and the frame decoder on arbitrary bytes. A header or frame
/// that decodes must encode back to the same bytes.
///
/// Cases: FUZZ-DECODER
pub fn decode_frame(data: &[u8]) {
    let Some((&mode, bytes)) = data.split_first() else {
        return;
    };
    let log = codec::LogId::new(LOG_ID);
    if let Some(header) = bytes.get(..codec::HEADER_LEN)
        && let Ok(decoded) = codec::decode_header(header)
    {
        assert_eq!(codec::encode_header(&decoded), header);
    }
    let mut frame = bytes.to_vec();
    if mode & 1 == 1 && frame.len() >= codec::ENVELOPE_LEN {
        let crc = log.frame_crc(&frame);
        frame[4..8].copy_from_slice(&crc.to_le_bytes());
    }
    let Ok(decoded) = codec::decode(&log, &frame) else {
        return;
    };
    let (epoch, base) = (decoded.epoch, decoded.base_offset);
    let encoded = match decoded.body {
        codec::Body::Group(entries) => {
            let group = codec::Group {
                epoch,
                base_offset: base,
                entries,
            };
            codec::encode_group(&log, &group).unwrap()
        }
        codec::Body::Footer(footer) => codec::encode_footer(&log, epoch, base, &footer).unwrap(),
        codec::Body::Seal => codec::encode_seal(&log, epoch, base),
        codec::Body::Locator { footer_position } => {
            codec::encode_locator(&log, epoch, base, footer_position)
        }
        codec::Body::KeyFilter(filter) => {
            let hash = u64::from_le_bytes(frame[frame.len() - 8..].try_into().unwrap());
            let _ = (filter.may_contain(hash), filter.index_pages(hash));
            return;
        }
        codec::Body::KeyIndex { count, pages } => {
            for (number, page) in pages.chunks(codec::PAGE_SIZE).enumerate() {
                let _ = codec::decode_page(page, number, count, 0);
            }
            return;
        }
    };
    assert_eq!(encoded, frame);
}

/// `FUZZ-RECOVERY`: open on a log whose only segment is a valid header followed by arbitrary
/// bytes. Open either fails with an error or keeps a valid prefix. A second open of what the first
/// one left finds the same records, and changes nothing.
///
/// Cases: FUZZ-RECOVERY
pub fn recover_active_segment(data: &[u8]) {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let log = codec::LogId::new(LOG_ID);
    let header = codec::Header {
        log_id: LOG_ID,
        base_offset: 0,
        epoch: 1,
    };
    let mut segment = codec::encode_header(&header).to_vec();
    segment.extend_from_slice(rest);
    if mode & 1 == 1 {
        checksum_frames(&log, &mut segment);
    }
    let sim = Sim::new(0);
    let dir = Path::new("/log");
    let path = segment_path(dir, 0);
    for (path, op) in [
        (dir, Op::CreateDir),
        (&path, Op::Create),
        (&path, Op::Write(0, segment.clone())),
        (&path, Op::SyncAll),
        (dir, Op::SyncDir),
    ] {
        call(&sim, path, Purpose::Open, op).unwrap();
    }
    let first = match recovery::open(&sim, dir, keys::KeyHash::new) {
        Ok(recovered) => summary(recovered),
        // A frame epoch at the top of the range leaves no epoch for the next writer.
        Err(OpenError::Corrupt { .. } | OpenError::Exhausted) => return,
        Err(error) => panic!("open of one active segment failed with {error:?}"),
    };
    let files = files(&sim, dir);
    assert!(
        files[0].len() <= segment.len() && segment.starts_with(&files[0]),
        "open kept bytes that were not in the segment"
    );
    let second =
        summary(recovery::open(&sim, dir, keys::KeyHash::new).expect("a second open fails"));
    assert_eq!(first, second, "a second open finds other records");
    assert_eq!(
        files,
        self::files(&sim, dir),
        "a second open changed the files"
    );
}

type Summary = (u64, Vec<(Vec<u8>, u64, u64)>);

fn summary(recovered: recovery::Recovered) -> Summary {
    let Published {
        watermark, heads, ..
    } = recovered.published;
    let heads = heads.into_iter().map(|(s, h)| (s, h.revision, h.offset));
    (watermark, heads.collect())
}

fn files(storage: &dyn Storage, dir: &Path) -> Vec<Vec<u8>> {
    let Ok(Value::Names(mut names)) = call(storage, dir, Purpose::Open, Op::List) else {
        panic!("list the log directory");
    };
    names.sort();
    let segments = names
        .iter()
        .filter(|n| n.to_string_lossy().ends_with(".seg"));
    segments
        .map(|name| {
            let read = Op::Read(0, usize::MAX);
            match call(storage, &dir.join(name), Purpose::Open, read) {
                Ok(Value::Bytes(bytes)) => bytes,
                other => panic!("read {name:?}: {other:?}"),
            }
        })
        .collect()
}

/// Rewrites the CRC of each frame whose length fits, from the first frame until one does not.
fn checksum_frames(log: &codec::LogId, segment: &mut [u8]) {
    let mut at = codec::HEADER_LEN;
    while let Some(len) = segment.get(at..at + 4) {
        let end = at + codec::ENVELOPE_LEN + u32::from_le_bytes(len.try_into().unwrap()) as usize;
        let Some(frame) = segment.get_mut(at..end) else {
            return;
        };
        let crc = log.frame_crc(frame);
        frame[4..8].copy_from_slice(&crc.to_le_bytes());
        at = end;
    }
}
