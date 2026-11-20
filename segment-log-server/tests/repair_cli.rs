//! `segment-log-server repair` on a real directory: exit codes and output of api.md's "Operator
//! repair" section (ADR 0065).
use segment_log_core::{ExpectedRevision, LogReader, NewRecord, Options, SegmentLog, Submit};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fresh(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("repair-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_segment-log-server"))
        .args(args)
        .output()
        .unwrap()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// A log of three records with a small roll size, so it has several segments.
fn write_log(dir: &Path) {
    let options = Options {
        roll_bytes: 100,
        linger: std::time::Duration::ZERO,
        ..Options::default()
    };
    let log = SegmentLog::open(dir, options).unwrap();
    for n in 0..3u8 {
        let submit = Submit {
            records: vec![NewRecord {
                stream: vec![b'a' + n],
                expected: ExpectedRevision::Empty,
                key: None,
                payload: vec![n; 40],
            }],
        };
        log.append(submit).unwrap().wait().unwrap();
    }
}

/// Given a healthy log, a log with a bad header in its oldest segment, a missing directory, a log
/// whose writer holds `LOCK`, and command lines with a missing subcommand or flag
/// When `segment-log-server repair verify` runs
/// Then it exits 0, 1, 5, 4 and 2, and `--json` prints schema 1
/// And the command line without a subcommand still serves, with its own exit status
///
/// Cases: REPAIR-CLI
#[test]
fn verify_exit_codes_and_json() {
    let dir = fresh("verify");
    write_log(&dir);
    let d = dir.to_str().unwrap();
    let healthy = run(&["repair", "verify", "--dir", d]);
    assert_eq!(code(&healthy), 0, "{healthy:?}");
    assert!(stdout(&healthy).ends_with("no damage\n"));
    let json = run(&["repair", "verify", "--dir", d, "--json"]);
    assert_eq!(code(&json), 0);
    assert_eq!(
        stdout(&json),
        "{\"schema\":1,\"command\":\"verify\",\"findings\":[],\"frames\":[],\"plan\":null,\"refusal\":null}\n"
    );

    let oldest = dir.join("00000000000000000000.seg");
    assert!(
        std::fs::read_dir(&dir).unwrap().count() > 2,
        "the log rolled"
    );
    let mut bytes = std::fs::read(&oldest).unwrap();
    bytes[30] ^= 1;
    std::fs::write(&oldest, &bytes).unwrap();
    let damaged = run(&["repair", "verify", "--dir", d, "--json"]);
    assert_eq!(code(&damaged), 1, "{damaged:?}");
    assert!(
        stdout(&damaged)
            .contains("\"findings\":[{\"class\":\"E\",\"segment\":0,\"position\":0,\"message\":\""),
        "{}",
        stdout(&damaged)
    );
    assert_eq!(
        std::fs::read(&oldest).unwrap(),
        bytes,
        "verify changed a segment"
    );

    let missing = run(&[
        "repair",
        "verify",
        "--dir",
        fresh("missing").to_str().unwrap(),
    ]);
    assert_eq!(code(&missing), 5, "{missing:?}");

    let held = fresh("held");
    let log = SegmentLog::open(&held, Options::default()).unwrap();
    let locked = run(&["repair", "verify", "--dir", held.to_str().unwrap()]);
    assert_eq!(code(&locked), 4, "{locked:?}");
    drop(log);

    assert_eq!(code(&run(&["repair", "verify"])), 2);
    assert_eq!(code(&run(&["repair", "mend", "--dir", d])), 2);
    assert_eq!(code(&run(&["repair"])), 2);
    assert_eq!(code(&run(&["--dir", d])), 1, "serve without a transport");
    for dir in [dir, held] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Given a sealed log whose second of three groups has a bad checksum, so open fails with `Corrupt`
/// When `segment-log-server repair truncate` runs without `--apply`
/// Then it exits 1 and prints a lossy plan that drops offsets 1 to 3
/// When `--apply` names another cut, or omits `--drop-acknowledged`
/// Then it exits 3 and changes nothing
/// When `--apply --at <cut> --drop-acknowledged` runs
/// Then it exits 0, names its `repair-<unix seconds>` backup, and the log reads as sealed with the
/// first record
/// And `truncate` on the repaired log exits 3, because there is nothing to cut
///
/// Cases: REPAIR-CLI
#[test]
fn truncate_exit_codes_and_json() {
    let dir = fresh("truncate");
    let log = SegmentLog::open(&dir, Options::default()).unwrap();
    for n in 0..3u8 {
        let submit = Submit {
            records: vec![NewRecord {
                stream: vec![b'a' + n],
                expected: ExpectedRevision::Empty,
                key: None,
                payload: vec![n; 200],
            }],
        };
        log.append(submit).unwrap().wait().unwrap();
    }
    log.seal().unwrap();
    drop(log);
    let segment = dir.join("00000000000000000000.seg");
    let mut bytes = std::fs::read(&segment).unwrap();
    // Header, then a group of 8 + 32 + 16 + 1 + 200 bytes; the byte sits in the second's body.
    let cut = 64 + 257;
    bytes[cut + 40] ^= 1;
    std::fs::write(&segment, &bytes).unwrap();
    let d = dir.to_str().unwrap();

    let dry = run(&["repair", "truncate", "--dir", d, "--json"]);
    assert_eq!(code(&dry), 1, "{dry:?}");
    let expected = format!(
        "\"plan\":{{\"cut\":\"0:{cut}\",\"kind\":\"lossy\",\"dropped_offsets\":[1,3],\
         \"closing_frames_removed\":[\"footer\",\"seal\",\"locator\"],\"backup\":null,\
         \"applied\":false}},\"refusal\":null}}"
    );
    assert!(
        stdout(&dry).ends_with(&format!("{expected}\n")),
        "{}",
        stdout(&dry)
    );
    assert!(
        stdout(&dry)
            .starts_with("{\"schema\":1,\"command\":\"truncate\",\"findings\":[{\"class\":\"A\"")
    );

    let at = format!("0:{cut}");
    let other = format!("0:{}", cut + 1);
    let mismatch = run(&["repair", "truncate", "--dir", d, "--apply", "--at", &other]);
    assert_eq!(code(&mismatch), 3, "{mismatch:?}");
    let undropped = run(&["repair", "truncate", "--dir", d, "--apply", "--at", &at]);
    assert_eq!(code(&undropped), 3, "{undropped:?}");
    assert!(stdout(&undropped).contains("refused: "));
    assert_eq!(
        std::fs::read(&segment).unwrap(),
        bytes,
        "a refusal changed the segment"
    );

    let applied = run(&[
        "repair",
        "truncate",
        "--dir",
        d,
        "--apply",
        "--at",
        &at,
        "--drop-acknowledged",
        "--json",
    ]);
    assert_eq!(code(&applied), 0, "{applied:?}");
    let out = stdout(&applied);
    assert!(out.contains("\"applied\":true"), "{out}");
    let backup = out
        .split("\"backup\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let name = Path::new(backup).file_name().unwrap().to_str().unwrap();
    assert!(
        name.strip_prefix("repair-").unwrap().parse::<u64>().is_ok(),
        "{name}"
    );
    assert_eq!(
        std::fs::read(Path::new(backup).join("00000000000000000000.seg")).unwrap(),
        bytes
    );

    assert_eq!(code(&run(&["repair", "verify", "--dir", d])), 0);
    let reader = LogReader::open(&dir).unwrap();
    assert_eq!(reader.read(0, 10).unwrap().len(), 1);
    assert_eq!(code(&run(&["repair", "truncate", "--dir", d])), 3);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Given an unsealed log whose second of three acknowledged groups has a flipped byte, so open
/// fails with `Corrupt` (ADR 0068)
/// When `segment-log-server repair truncate --json` runs without `--apply`
/// Then it exits 1, names class F, and prints a lossy plan that drops offsets 1 to 3 and removes
/// no closing frame
/// When `--apply` omits `--drop-acknowledged`
/// Then it exits 3 and changes nothing
/// When `--apply --at <cut> --drop-acknowledged` runs
/// Then it exits 0, and the log reads as sealed with the first record
///
/// Cases: REPAIR-CLASS-F-CLI
#[test]
fn truncate_class_f_exit_codes_and_json() {
    let dir = fresh("class-f");
    let log = SegmentLog::open(&dir, Options::default()).unwrap();
    for n in 0..3u8 {
        let submit = Submit {
            records: vec![NewRecord {
                stream: vec![b'a' + n],
                expected: ExpectedRevision::Empty,
                key: None,
                payload: vec![n; 200],
            }],
        };
        log.append(submit).unwrap().wait().unwrap();
    }
    drop(log);
    let segment = dir.join("00000000000000000000.seg");
    let mut bytes = std::fs::read(&segment).unwrap();
    // Header, then a group of 8 + 32 + 16 + 1 + 200 bytes; the byte sits in the second's body.
    let cut = 64 + 257;
    bytes[cut + 40] ^= 1;
    std::fs::write(&segment, &bytes).unwrap();
    let d = dir.to_str().unwrap();
    assert!(matches!(
        SegmentLog::open(&dir, Options::default()),
        Err(segment_log_core::OpenError::Corrupt { segment: 0, position }) if position == cut as u64
    ));

    let dry = run(&["repair", "truncate", "--dir", d, "--json"]);
    assert_eq!(code(&dry), 1, "{dry:?}");
    assert!(
        stdout(&dry)
            .starts_with("{\"schema\":1,\"command\":\"truncate\",\"findings\":[{\"class\":\"F\""),
        "{}",
        stdout(&dry)
    );
    let expected = format!(
        "\"plan\":{{\"cut\":\"0:{cut}\",\"kind\":\"lossy\",\"dropped_offsets\":[1,3],\
         \"closing_frames_removed\":[],\"backup\":null,\"applied\":false}},\"refusal\":null}}"
    );
    assert!(
        stdout(&dry).ends_with(&format!("{expected}\n")),
        "{}",
        stdout(&dry)
    );

    let at = format!("0:{cut}");
    let undropped = run(&["repair", "truncate", "--dir", d, "--apply", "--at", &at]);
    assert_eq!(code(&undropped), 3, "{undropped:?}");
    assert_eq!(
        std::fs::read(&segment).unwrap(),
        bytes,
        "a refusal changed the segment"
    );

    let applied = run(&[
        "repair",
        "truncate",
        "--dir",
        d,
        "--apply",
        "--at",
        &at,
        "--drop-acknowledged",
    ]);
    assert_eq!(code(&applied), 0, "{applied:?}");
    let reader = LogReader::open(&dir).unwrap();
    assert_eq!(reader.read(0, 10).unwrap().len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}
