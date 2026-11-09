use super::*;
use crate::acceptance::support::trace::{bytes, number, read_bytes, read_number};
use std::io::{Read, Write};

fn input(w: &mut impl Write, r: &Input) {
    bytes(w, &r.stream).unwrap();
    number(w, u64::from(r.key.is_some())).unwrap();
    if let Some(k) = &r.key {
        bytes(w, k).unwrap();
    }
    bytes(w, &r.payload).unwrap();
    number(w, u64::from(r.expected.is_some())).unwrap();
    if let Some(n) = r.expected {
        number(w, n).unwrap();
    }
    number(w, r.spare as u64).unwrap();
}
fn n(r: &mut impl Read) -> u64 {
    read_number(r).unwrap()
}
fn b(r: &mut impl Read) -> Vec<u8> {
    read_bytes(r).unwrap()
}
fn count(r: &mut impl Read) -> usize {
    let v = n(r);
    assert!(v <= 1_000_000);
    v as usize
}
fn read_input(r: &mut impl Read) -> Input {
    Input {
        stream: b(r),
        key: if n(r) == 1 { Some(b(r)) } else { None },
        payload: b(r),
        expected: if n(r) == 1 { Some(n(r)) } else { None },
        spare: usize::try_from(n(r)).unwrap(),
    }
}
fn answer(w: &mut impl Write, a: &Answer) {
    match a {
        Answer::Unknown => number(w, 0).unwrap(),
        Answer::Refused(s) => {
            number(w, 1).unwrap();
            bytes(w, s.as_bytes()).unwrap();
        }
        Answer::Ack(d, ps) => {
            number(w, 2).unwrap();
            number(w, u64::from(*d)).unwrap();
            number(w, ps.len() as u64).unwrap();
            for (o, v) in ps {
                number(w, *o).unwrap();
                number(w, *v).unwrap();
            }
        }
    }
}
fn read_answer(r: &mut impl Read) -> Answer {
    match n(r) {
        0 => Answer::Unknown,
        1 => Answer::Refused(String::from_utf8(b(r)).unwrap()),
        2 => {
            let d = n(r) == 1;
            Answer::Ack(d, (0..count(r)).map(|_| (n(r), n(r))).collect())
        }
        _ => panic!("invalid ledger answer"),
    }
}
pub(super) fn save(path: &Path, ledger: &Ledger) {
    let mut w = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    w.write_all(b"SLLEDGR1").unwrap();
    for v in [
        ledger.seed,
        ledger.mode as u64,
        ledger.options.group_byte_cap as u64,
        ledger.options.queue_capacity as u64,
        ledger.options.pending_byte_budget as u64,
        ledger.options.linger.as_nanos() as u64,
        ledger.options.roll_bytes,
    ] {
        number(&mut w, v).unwrap();
    }
    number(&mut w, ledger.requests.len() as u64).unwrap();
    for q in &ledger.requests {
        number(&mut w, u64::from(q.original)).unwrap();
        number(&mut w, q.watermark).unwrap();
        answer(&mut w, &q.answer);
        number(&mut w, q.inputs.len() as u64).unwrap();
        for r in &q.inputs {
            input(&mut w, r);
        }
    }
    number(&mut w, ledger.reads.len() as u64).unwrap();
    for o in &ledger.reads {
        for v in [
            o.before,
            o.after,
            o.from,
            o.max as u64,
            o.records.len() as u64,
        ] {
            number(&mut w, v).unwrap();
        }
        for r in &o.records {
            input(
                &mut w,
                &Input {
                    stream: r.stream.clone(),
                    key: r.key.clone(),
                    payload: r.payload.clone(),
                    expected: None,
                    spare: 0,
                },
            );
            number(&mut w, r.offset).unwrap();
            number(&mut w, r.revision).unwrap();
        }
        number(&mut w, o.heads.len() as u64).unwrap();
        for (s, h) in &o.heads {
            bytes(&mut w, s).unwrap();
            number(&mut w, u64::from(h.is_some())).unwrap();
            if let Some((a, b)) = h {
                number(&mut w, *a).unwrap();
                number(&mut w, *b).unwrap();
            }
        }
    }
    w.flush().unwrap();
}
pub(super) fn load(path: &Path) -> Ledger {
    let mut r = std::io::BufReader::new(std::fs::File::open(path).unwrap());
    let mut magic = [0; 8];
    r.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, b"SLLEDGR1");
    let seed = n(&mut r);
    let mode = match n(&mut r) {
        0 => Crash::Kill,
        1 => Crash::PowerLoss,
        2 => Crash::PowerLossTorn,
        _ => panic!("mode"),
    };
    let options = Options {
        group_byte_cap: n(&mut r) as usize,
        queue_capacity: count(&mut r),
        pending_byte_budget: n(&mut r) as usize,
        linger: Duration::from_nanos(n(&mut r)),
        roll_bytes: n(&mut r),
    };
    let requests = (0..count(&mut r))
        .map(|_| {
            let original = n(&mut r) == 1;
            let watermark = n(&mut r);
            let answer = read_answer(&mut r);
            let inputs = (0..count(&mut r)).map(|_| read_input(&mut r)).collect();
            Request {
                original,
                watermark,
                answer,
                inputs,
            }
        })
        .collect();
    let reads = (0..count(&mut r))
        .map(|_| {
            let before = n(&mut r);
            let after = n(&mut r);
            let from = n(&mut r);
            let max = n(&mut r) as usize;
            let records = (0..count(&mut r))
                .map(|_| {
                    let input = read_input(&mut r);
                    input.stored(n(&mut r), n(&mut r))
                })
                .collect();
            let heads = (0..count(&mut r))
                .map(|_| {
                    let s = b(&mut r);
                    let h = if n(&mut r) == 1 {
                        Some((n(&mut r), n(&mut r)))
                    } else {
                        None
                    };
                    (s, h)
                })
                .collect();
            Observation {
                before,
                after,
                from,
                max,
                records,
                heads,
            }
        })
        .collect();
    let mut trailing = [0];
    assert_eq!(r.read(&mut trailing).unwrap(), 0);
    Ledger {
        seed,
        mode,
        options,
        requests,
        reads,
    }
}
