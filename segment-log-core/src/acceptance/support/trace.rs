use crate::storage::{
    Call, Op, Purpose, Value,
    sim::{Crash, ERROR_KINDS as KINDS, Event, Fault, Outcome},
};
use std::{
    io::{self, Read, Write},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
pub(crate) fn number(w: &mut impl Write, n: u64) -> io::Result<()> {
    w.write_all(&n.to_le_bytes())
}
pub(crate) fn bytes(w: &mut impl Write, b: &[u8]) -> io::Result<()> {
    number(w, b.len() as u64)?;
    w.write_all(b)
}
pub(crate) fn read_number(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
pub(crate) fn read_bytes(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let len = usize::try_from(read_number(r)?).map_err(|_| invalid())?;
    if len > 64 * 1024 * 1024 {
        return Err(invalid());
    }
    let mut b = vec![0; len];
    r.read_exact(&mut b)?;
    Ok(b)
}
fn invalid() -> io::Error {
    io::Error::from(io::ErrorKind::InvalidData)
}
fn path(w: &mut impl Write, p: &Path) -> io::Result<()> {
    bytes(w, p.as_os_str().as_bytes())
}
fn read_path(r: &mut impl Read) -> io::Result<PathBuf> {
    Ok(std::ffi::OsString::from_vec(read_bytes(r)?).into())
}

fn kind(w: &mut impl Write, k: io::ErrorKind) -> io::Result<()> {
    number(
        w,
        KINDS.iter().position(|x| *x == k).ok_or_else(invalid)? as u64,
    )
}
fn read_kind(r: &mut impl Read) -> io::Result<io::ErrorKind> {
    KINDS
        .get(read_number(r)? as usize)
        .copied()
        .ok_or_else(invalid)
}
fn purpose(w: &mut impl Write, purpose: Purpose) -> io::Result<()> {
    number(
        w,
        match purpose {
            Purpose::Open => 0,
            Purpose::Group => 1,
            Purpose::Recovery => 2,
            Purpose::Lookup => 3,
            Purpose::Roll => 4,
        },
    )
}
fn read_purpose(r: &mut impl Read) -> io::Result<Purpose> {
    Ok(match read_number(r)? {
        0 => Purpose::Open,
        1 => Purpose::Group,
        2 => Purpose::Recovery,
        3 => Purpose::Lookup,
        4 => Purpose::Roll,
        _ => return Err(invalid()),
    })
}
fn op(w: &mut impl Write, op: &Op) -> io::Result<()> {
    let tag = match op {
        Op::CreateDir => 0,
        Op::Create => 1,
        Op::Open => 2,
        Op::Append(_) => 3,
        Op::Write(..) => 4,
        Op::Read(..) => 5,
        Op::SyncData => 6,
        Op::SyncAll => 7,
        Op::SyncDir => 8,
        Op::Truncate(_) => 9,
        Op::Remove => 10,
        Op::List => 11,
        Op::Random(_) => 12,
    };
    number(w, tag)?;
    match op {
        Op::Append(b) => bytes(w, b)?,
        Op::Write(p, b) => {
            number(w, *p)?;
            bytes(w, b)?;
        }
        Op::Read(p, n) => {
            number(w, *p)?;
            number(w, *n as u64)?;
        }
        Op::Truncate(n) => number(w, *n)?,
        Op::Random(n) => number(w, *n as u64)?,
        _ => {}
    }
    Ok(())
}
fn read_op(r: &mut impl Read) -> io::Result<Op> {
    Ok(match read_number(r)? {
        0 => Op::CreateDir,
        1 => Op::Create,
        2 => Op::Open,
        3 => Op::Append(read_bytes(r)?),
        4 => Op::Write(read_number(r)?, read_bytes(r)?),
        5 => Op::Read(read_number(r)?, read_number(r)? as usize),
        6 => Op::SyncData,
        7 => Op::SyncAll,
        8 => Op::SyncDir,
        9 => Op::Truncate(read_number(r)?),
        10 => Op::Remove,
        11 => Op::List,
        12 => Op::Random(read_number(r)? as usize),
        _ => return Err(invalid()),
    })
}
fn fault(w: &mut impl Write, f: &Option<Fault>) -> io::Result<()> {
    match f {
        None => number(w, 0),
        Some(Fault::Error(k)) => {
            number(w, 1)?;
            kind(w, k.kind())
        }
        Some(Fault::Partial(n)) => {
            number(w, 2)?;
            number(w, *n as u64)
        }
        Some(Fault::NoSpace) => number(w, 3),
        Some(Fault::Corrupt(n, x)) => {
            number(w, 4)?;
            number(w, *n as u64)?;
            number(w, u64::from(*x))
        }
        Some(Fault::Eof) => number(w, 5),
        Some(Fault::Panic) => number(w, 6),
    }
}
fn read_fault(r: &mut impl Read) -> io::Result<Option<Fault>> {
    Ok(match read_number(r)? {
        0 => None,
        1 => Some(Fault::error(read_kind(r)?).map_err(|_| invalid())?),
        2 => Some(Fault::Partial(read_number(r)? as usize)),
        3 => Some(Fault::NoSpace),
        4 => Some(Fault::Corrupt(
            read_number(r)? as usize,
            read_number(r)? as u8,
        )),
        5 => Some(Fault::Eof),
        6 => Some(Fault::Panic),
        _ => return Err(invalid()),
    })
}
fn outcome(w: &mut impl Write, o: &Outcome) -> io::Result<()> {
    match o {
        Outcome::Panic => number(w, 0),
        Outcome::Error(k) => {
            number(w, 1)?;
            kind(w, *k)
        }
        Outcome::Ok(Value::Unit) => number(w, 2),
        Outcome::Ok(Value::Length(n)) => {
            number(w, 3)?;
            number(w, *n)
        }
        Outcome::Ok(Value::Bytes(b)) => {
            number(w, 4)?;
            bytes(w, b)
        }
        Outcome::Ok(Value::Names(names)) => {
            number(w, 5)?;
            number(w, names.len() as u64)?;
            for name in names {
                path(w, name)?;
            }
            Ok(())
        }
    }
}
fn read_outcome(r: &mut impl Read) -> io::Result<Outcome> {
    Ok(match read_number(r)? {
        0 => Outcome::Panic,
        1 => Outcome::Error(read_kind(r)?),
        2 => Outcome::Ok(Value::Unit),
        3 => Outcome::Ok(Value::Length(read_number(r)?)),
        4 => Outcome::Ok(Value::Bytes(read_bytes(r)?)),
        5 => {
            let n = read_number(r)?;
            if n > 1_000_000 {
                return Err(invalid());
            }
            let mut names = vec![];
            for _ in 0..n {
                names.push(read_path(r)?);
            }
            Outcome::Ok(Value::Names(names))
        }
        _ => return Err(invalid()),
    })
}
pub(crate) fn save(file: &Path, seed: u64, trace: &[Event]) -> io::Result<()> {
    let mut w = std::io::BufWriter::new(std::fs::File::create(file)?);
    w.write_all(b"SLTRACE1")?;
    number(&mut w, seed)?;
    number(&mut w, trace.len() as u64)?;
    for event in trace {
        match event {
            Event::Call(c, f, o) => {
                number(&mut w, 0)?;
                path(&mut w, &c.path)?;
                purpose(&mut w, c.purpose)?;
                op(&mut w, &c.op)?;
                fault(&mut w, f)?;
                outcome(&mut w, o)?;
            }
            Event::Canceled(c) => {
                number(&mut w, 4)?;
                path(&mut w, &c.path)?;
                purpose(&mut w, c.purpose)?;
                op(&mut w, &c.op)?;
            }
            Event::Lock(p, b) => {
                number(&mut w, 1)?;
                path(&mut w, p)?;
                number(&mut w, u64::from(*b))?;
            }
            Event::Unlock(id) => {
                number(&mut w, 2)?;
                number(&mut w, *id)?;
            }
            Event::Crash(mode, seed) => {
                number(&mut w, 3)?;
                number(
                    &mut w,
                    match mode {
                        Crash::Kill => 0,
                        Crash::PowerLoss => 1,
                        Crash::PowerLossTorn => 2,
                    },
                )?;
                number(&mut w, *seed)?;
            }
        }
    }
    w.flush()
}
pub(crate) fn load(file: &Path) -> io::Result<(u64, Vec<Event>)> {
    let mut r = std::io::BufReader::new(std::fs::File::open(file)?);
    let mut magic = [0; 8];
    r.read_exact(&mut magic)?;
    if &magic != b"SLTRACE1" {
        return Err(invalid());
    }
    let seed = read_number(&mut r)?;
    let n = read_number(&mut r)?;
    if n > 1_000_000 {
        return Err(invalid());
    }
    let mut trace = vec![];
    for _ in 0..n {
        trace.push(match read_number(&mut r)? {
            0 => {
                let path = read_path(&mut r)?;
                let purpose = read_purpose(&mut r)?;
                let op = read_op(&mut r)?;
                Event::Call(
                    Call { path, purpose, op },
                    read_fault(&mut r)?,
                    read_outcome(&mut r)?,
                )
            }
            4 => {
                let path = read_path(&mut r)?;
                let purpose = read_purpose(&mut r)?;
                Event::Canceled(Call {
                    path,
                    purpose,
                    op: read_op(&mut r)?,
                })
            }
            1 => Event::Lock(read_path(&mut r)?, read_number(&mut r)? != 0),
            2 => Event::Unlock(read_number(&mut r)?),
            3 => {
                let mode = match read_number(&mut r)? {
                    0 => Crash::Kill,
                    1 => Crash::PowerLoss,
                    2 => Crash::PowerLossTorn,
                    _ => return Err(invalid()),
                };
                Event::Crash(mode, read_number(&mut r)?)
            }
            _ => return Err(invalid()),
        });
    }
    let mut trailing = [0];
    if r.read(&mut trailing)? != 0 {
        return Err(invalid());
    }
    Ok((seed, trace))
}

/// Given a recorded storage failure outside the original trace error subset
/// When the trace is saved and loaded
/// Then replay retains the exact fault and observed error kind
/// Cases: SIM-MODEL
#[test]
fn saving_a_trace_preserves_broken_pipe_errors() {
    super::bounded(|| {
        let event = Event::Call(
            Call::new("/log/a", Purpose::Group, Op::SyncData),
            Some(Fault::error(io::ErrorKind::BrokenPipe).unwrap()),
            Outcome::Error(io::ErrorKind::BrokenPipe),
        );
        let file =
            std::env::temp_dir().join(format!("segment-log-error-kinds-{}", std::process::id()));
        let result = save(&file, 1, std::slice::from_ref(&event));
        assert!(
            result.is_ok(),
            "every accepted fault must be persistable: {result:?}"
        );
        assert_eq!(load(&file).unwrap(), (1, vec![event]));
        std::fs::remove_file(file).unwrap();
    });
}
