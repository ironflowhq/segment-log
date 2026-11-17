//! `segment-log-server repair`: the operator repair command of ADR 0065. api.md's "Operator
//! repair" section is the only source for its flags, exit codes and JSON output.

use segment_log_core::repair::{self, Error, Finding, FrameInfo, Report};
use std::path::PathBuf;
use std::process::ExitCode;

pub const USAGE: &str = "usage: segment-log-server repair verify --dir <path> [--json]";

const EXIT_DAMAGE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_LOCKED: u8 = 4;
const EXIT_IO: u8 = 5;

struct Command {
    dir: PathBuf,
    json: bool,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    match args.next().as_deref() {
        Some("verify") => (),
        Some(other) => return Err(format!("unknown repair subcommand {other}")),
        None => return Err("give a repair subcommand".into()),
    }
    let (mut dir, mut json) = (None, false);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--json" => json = true,
            "--dir" => dir = Some(PathBuf::from(args.next().ok_or("--dir needs a value")?)),
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    Ok(Command {
        dir: dir.ok_or("--dir is required")?,
        json,
    })
}

pub fn main(args: impl Iterator<Item = String>) -> ExitCode {
    let command = match parse(args) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("segment-log-server repair: {error}\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let report = match repair::verify(&command.dir) {
        Ok(report) => report,
        Err(Error::Locked) => {
            eprintln!(
                "segment-log-server repair: {}: another process holds LOCK",
                command.dir.display()
            );
            return ExitCode::from(EXIT_LOCKED);
        }
        Err(Error::Io(error)) => {
            eprintln!(
                "segment-log-server repair: {}: {error}",
                command.dir.display()
            );
            return ExitCode::from(EXIT_IO);
        }
    };
    if command.json {
        println!("{}", json(&report));
    } else {
        print!("{}", text(&report));
    }
    ExitCode::from(if report.damaged() { EXIT_DAMAGE } else { 0 })
}

fn text(report: &Report) -> String {
    let mut out = String::new();
    for f in &report.findings {
        out += &format!(
            "{} segment {} position {}: {}\n",
            f.class.label(),
            f.segment,
            f.position,
            f.message
        );
    }
    for f in &report.frames {
        out += &format!(
            "  frame segment {} position {}: {} epoch {} base offset {} count {}{}\n",
            f.segment,
            f.position,
            f.kind,
            f.epoch,
            f.base_offset,
            f.count,
            if f.valid { "" } else { " (not valid)" }
        );
    }
    out += if report.damaged() {
        "damage found\n"
    } else {
        "no damage\n"
    };
    out
}

fn json(report: &Report) -> String {
    let findings: Vec<_> = report.findings.iter().map(finding).collect();
    let frames: Vec<_> = report.frames.iter().map(frame).collect();
    format!(
        "{{\"schema\":1,\"command\":\"verify\",\"findings\":[{}],\"frames\":[{}],\"plan\":null}}",
        findings.join(","),
        frames.join(",")
    )
}

fn finding(f: &Finding) -> String {
    format!(
        "{{\"class\":\"{}\",\"segment\":{},\"position\":{},\"message\":{}}}",
        f.class.label(),
        f.segment,
        f.position,
        string(&f.message)
    )
}

fn frame(f: &FrameInfo) -> String {
    format!(
        "{{\"segment\":{},\"position\":{},\"kind\":\"{}\",\"epoch\":{},\"base_offset\":{},\"count\":{},\"valid\":{}}}",
        f.segment, f.position, f.kind, f.epoch, f.base_offset, f.count, f.valid
    )
}

fn string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            c if (c as u32) < 0x20 => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_strings_escape_quotes_backslashes_and_controls() {
        assert_eq!(string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\u000ad\"");
    }

    #[test]
    fn parse_needs_a_subcommand_and_a_dir() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse(args(&[]).into_iter()).is_err());
        assert!(parse(args(&["verify"]).into_iter()).is_err());
        assert!(parse(args(&["fix", "--dir", "d"]).into_iter()).is_err());
        let command = parse(args(&["verify", "--dir", "d", "--json"]).into_iter()).unwrap();
        assert_eq!((command.dir, command.json), (PathBuf::from("d"), true));
    }
}
