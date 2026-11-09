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
