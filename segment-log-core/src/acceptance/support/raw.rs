use super::fixtures::{SEG, Sim, open};
use crate::storage::{Call, Op, Purpose, Storage, Value};

pub(crate) fn crc(bytes: &[u8]) -> u32 {
    let mut state = u32::MAX;
    for byte in bytes {
        state ^= u32::from(*byte);
        for _ in 0..8 {
            state = if state & 1 == 1 {
                (state >> 1) ^ 0x82f63b78
            } else {
                state >> 1
            };
        }
    }
    state ^ u32::MAX
}
pub(crate) fn header(version: u16, base: u64, epoch: u64) -> Vec<u8> {
    assert_eq!(crc(b"123456789"), 0xe3069283);
    let mut bytes = vec![0; 64];
    bytes[..4].copy_from_slice(b"SGLG");
    bytes[4..6].copy_from_slice(&version.to_le_bytes());
    bytes[6..8].copy_from_slice(&64u16.to_le_bytes());
    bytes[8..24].fill(7);
    bytes[24..32].copy_from_slice(&base.to_le_bytes());
    bytes[32..40].copy_from_slice(&epoch.to_le_bytes());
    let checksum = crc(&bytes[..60]);
    bytes[60..64].copy_from_slice(&checksum.to_le_bytes());
    bytes
}
pub(crate) fn group(epoch: u64, base: u64, payloads: &[&[u8]]) -> Vec<u8> {
    let mut body = vec![1, 0, 0, 0];
    body.extend(epoch.to_le_bytes());
    body.extend(base.to_le_bytes());
    body.extend((payloads.len() as u32).to_le_bytes());
    for (i, payload) in payloads.iter().enumerate() {
        body.extend(1u16.to_le_bytes());
        body.extend(0u16.to_le_bytes());
        body.extend((payload.len() as u32).to_le_bytes());
        body.extend((base + i as u64).to_le_bytes());
        body.push(b's');
        body.extend(*payload);
    }
    let mut input = (body.len() as u32).to_le_bytes().to_vec();
    input.extend(&body);
    let mut frame = input[..4].to_vec();
    frame.extend(crc(&input).to_le_bytes());
    frame.extend(body);
    frame
}
pub(crate) fn install(sim: &Sim, bytes: Vec<u8>) {
    drop(open(sim));
    sim.call(Call::new(SEG, Purpose::Open, Op::Truncate(0)))
        .unwrap();
    sim.call(Call::new(SEG, Purpose::Open, Op::Write(0, bytes)))
        .unwrap();
    sim.call(Call::new(SEG, Purpose::Open, Op::SyncAll))
        .unwrap();
}
pub(crate) fn bytes(sim: &Sim) -> Vec<u8> {
    let Value::Bytes(bytes) = sim
        .call(Call::new(SEG, Purpose::Open, Op::Read(0, 1024 * 1024)))
        .unwrap()
    else {
        panic!("read bytes")
    };
    bytes
}
/// The same header under another log id. `header` fills the id with 7.
pub(crate) fn with_log_id(mut header: Vec<u8>, log_id: [u8; 16]) -> Vec<u8> {
    header[8..24].copy_from_slice(&log_id);
    let checksum = crc(&header[..60]);
    header[60..64].copy_from_slice(&checksum.to_le_bytes());
    header
}
/// The log id, base offset and epoch of a segment image whose header checksum matches.
pub(crate) fn header_fields(segment: &[u8]) -> ([u8; 16], u64, u64) {
    assert_eq!(
        crc(&segment[..60]),
        u32::from_le_bytes(segment[60..64].try_into().unwrap())
    );
    (
        segment[8..24].try_into().unwrap(),
        u64::from_le_bytes(segment[24..32].try_into().unwrap()),
        u64::from_le_bytes(segment[32..40].try_into().unwrap()),
    )
}
/// The frames of a segment image. Panics unless the image is a header followed by whole frames.
pub(crate) fn frames(segment: &[u8]) -> Vec<&[u8]> {
    let mut frames = vec![];
    let mut at = 64;
    while at < segment.len() {
        let length = u32::from_le_bytes(segment[at..at + 4].try_into().unwrap()) as usize + 8;
        frames.push(&segment[at..at + length]);
        at += length;
    }
    frames
}
pub(crate) fn segment(base: u64) -> String {
    format!("/log/{base:020}.seg")
}
/// Replaces the segment files of the log with `segments`, each named by its base offset.
pub(crate) fn install_segments(sim: &Sim, segments: &[(u64, Vec<u8>)]) {
    drop(open(sim));
    for (base, _) in self::segments(sim) {
        sim.call(Call::new(segment(base), Purpose::Open, Op::Remove))
            .unwrap();
    }
    for (base, bytes) in segments {
        let path = segment(*base);
        sim.call(Call::new(&path, Purpose::Open, Op::Create))
            .unwrap();
        sim.call(Call::new(&path, Purpose::Open, Op::Write(0, bytes.clone())))
            .unwrap();
        sim.call(Call::new(&path, Purpose::Open, Op::SyncAll))
            .unwrap();
    }
    sim.call(Call::new("/log", Purpose::Open, Op::SyncDir))
        .unwrap();
}
/// Every segment file of the log with its bytes, oldest first. A log directory that does not exist
/// yet has none.
pub(crate) fn segments(sim: &Sim) -> Vec<(u64, Vec<u8>)> {
    let names = match sim.call(Call::new("/log", Purpose::Open, Op::List)) {
        Ok(Value::Names(names)) => names,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return vec![],
        other => panic!("list /log: {other:?}"),
    };
    let mut bases: Vec<u64> = names
        .iter()
        .filter_map(|name| name.to_str()?.strip_suffix(".seg")?.parse().ok())
        .collect();
    bases.sort_unstable();
    bases
        .into_iter()
        .map(|base| {
            let read = Call::new(segment(base), Purpose::Open, Op::Read(0, usize::MAX));
            let Value::Bytes(bytes) = sim.call(read).unwrap() else {
                panic!("read bytes")
            };
            (base, bytes)
        })
        .collect()
}
/// The path, position and bytes of the last frame in the newest segment.
pub(crate) fn last_frame(sim: &Sim) -> (String, u64, Vec<u8>) {
    let (base, bytes) = segments(sim).pop().expect("a segment");
    let last = frames(&bytes).pop().expect("a frame").to_vec();
    let position = bytes.len() - last.len();
    (segment(base), position as u64, last)
}
