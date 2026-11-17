//! `segment-log-server repair` on a real directory: exit codes and output of api.md's "Operator
//! repair" section (ADR 0065).
use segment_log_core::{ExpectedRevision, NewRecord, Options, SegmentLog, Submit};
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
        "{\"schema\":1,\"command\":\"verify\",\"findings\":[],\"frames\":[],\"plan\":null}\n"
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
