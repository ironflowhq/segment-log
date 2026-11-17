//! `segment-log-server repair`: the operator repair command of ADR 0065. api.md's "Operator
//! repair" section is the only source for its flags, exit codes and JSON output.

use segment_log_core::repair::{self, Apply, Error, Finding, FrameInfo, Outcome, Plan};
use std::path::PathBuf;
use std::process::ExitCode;

pub const USAGE: &str = "usage: segment-log-server repair verify --dir <path> [--json]
       segment-log-server repair truncate --dir <path> [--apply --at <segment>:<position> \
[--drop-acknowledged]] [--json]";

const EXIT_DAMAGE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_REFUSED: u8 = 3;
const EXIT_LOCKED: u8 = 4;
const EXIT_IO: u8 = 5;

struct Command {
    truncate: bool,
    dir: PathBuf,
    json: bool,
    /// `--apply` with its `--at`.
    at: Option<(u64, u64)>,
    drop_acknowledged: bool,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let truncate = match args.next().as_deref() {
        Some("verify") => false,
        Some("truncate") => true,
        Some(other) => return Err(format!("unknown repair subcommand {other}")),
        None => return Err("give a repair subcommand".into()),
    };
    let (mut dir, mut json, mut apply, mut at, mut drop_acknowledged) =
        (None, false, false, None, false);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--json" => json = true,
            "--dir" => dir = Some(PathBuf::from(args.next().ok_or("--dir needs a value")?)),
            "--apply" if truncate => apply = true,
            "--drop-acknowledged" if truncate => drop_acknowledged = true,
            "--at" if truncate => {
                let value = args.next().ok_or("--at needs a value")?;
                at = Some(cut(&value).ok_or(format!("--at: not <segment>:<position>: {value}"))?);
            }
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    if apply != at.is_some() || (drop_acknowledged && !apply) {
        return Err("--apply needs --at, and --at and --drop-acknowledged need --apply".into());
    }
    Ok(Command {
        truncate,
        dir: dir.ok_or("--dir is required")?,
        json,
        at,
        drop_acknowledged,
    })
}

fn cut(value: &str) -> Option<(u64, u64)> {
    let (segment, position) = value.split_once(':')?;
    Some((segment.parse().ok()?, position.parse().ok()?))
}

pub fn main(args: impl Iterator<Item = String>) -> ExitCode {
    let command = match parse(args) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("segment-log-server repair: {error}\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let result = match (command.truncate, command.at) {
        (false, _) => repair::verify(&command.dir).map(|report| Outcome {
            report,
            plan: None,
            refusal: None,
        }),
        (true, None) => repair::plan(&command.dir),
        (true, Some(at)) => {
            // The name ADR 0065 gives the backup directory.
            let seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let backup = format!("repair-{seconds}");
            let apply = Apply {
                at,
                drop_acknowledged: command.drop_acknowledged,
                backup: &backup,
            };
            repair::apply(&command.dir, apply)
        }
    };
    let outcome = match result {
        Ok(outcome) => outcome,
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
    let name = if command.truncate {
        "truncate"
    } else {
        "verify"
    };
    if command.json {
        println!("{}", json(name, &outcome));
    } else {
        print!("{}", text(&outcome));
    }
    let code = match (&outcome.refusal, &outcome.plan) {
        (Some(_), _) => EXIT_REFUSED,
        (None, Some(plan)) if plan.applied => 0,
        (None, Some(_)) => EXIT_DAMAGE,
        (None, None) if outcome.report.damaged() => EXIT_DAMAGE,
        (None, None) => 0,
    };
    ExitCode::from(code)
}

fn text(outcome: &Outcome) -> String {
    let report = &outcome.report;
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
    if let Some(p) = &outcome.plan {
        out += &format!(
            "plan: {} cut at {}:{}, removing closing frames {}\n",
            p.kind.label(),
            p.segment,
            p.cut,
            p.closing_frames_removed.join(", ")
        );
        if let Some((from, to)) = p.dropped_offsets {
            out += &format!("plan drops acknowledged offsets {from} to {to} (half-open)\n");
        }
        if let Some(backup) = &p.backup {
            out += &format!("backup: {}\n", backup.display());
        }
        if p.applied {
            out += "applied\n";
        } else if outcome.refusal.is_none() {
            out += &format!(
                "dry run: repeat with --apply --at {}:{}\n",
                p.segment, p.cut
            );
        }
    }
    if let Some(refusal) = &outcome.refusal {
        out += &format!("refused: {refusal}\n");
    }
    out
}

fn json(command: &str, outcome: &Outcome) -> String {
    let report = &outcome.report;
    let findings: Vec<_> = report.findings.iter().map(finding).collect();
    let frames: Vec<_> = report.frames.iter().map(frame).collect();
    format!(
        "{{\"schema\":1,\"command\":\"{command}\",\"findings\":[{}],\"frames\":[{}],\"plan\":{},\"refusal\":{}}}",
        findings.join(","),
        frames.join(","),
        outcome.plan.as_ref().map_or("null".into(), plan),
        outcome.refusal.as_deref().map_or("null".into(), string)
    )
}

fn plan(p: &Plan) -> String {
    let removed: Vec<_> = p
        .closing_frames_removed
        .iter()
        .map(|k| format!("\"{k}\""))
        .collect();
    format!(
        "{{\"cut\":\"{}:{}\",\"kind\":\"{}\",\"dropped_offsets\":{},\"closing_frames_removed\":[{}],\"backup\":{},\"applied\":{}}}",
        p.segment,
        p.cut,
        p.kind.label(),
        p.dropped_offsets
            .map_or("null".into(), |(a, b)| format!("[{a},{b}]")),
        removed.join(","),
        p.backup
            .as_ref()
            .map_or("null".into(), |b| string(&b.display().to_string())),
        p.applied
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
        assert!(parse(args(&["verify", "--dir", "d", "--apply"]).into_iter()).is_err());
        assert!(parse(args(&["truncate", "--dir", "d", "--apply"]).into_iter()).is_err());
        assert!(parse(args(&["truncate", "--dir", "d", "--at", "0:64"]).into_iter()).is_err());
        assert!(
            parse(args(&["truncate", "--dir", "d", "--drop-acknowledged"]).into_iter()).is_err()
        );
        assert!(
            parse(args(&["truncate", "--dir", "d", "--apply", "--at", "0"]).into_iter()).is_err()
        );
        let full = [
            "truncate",
            "--dir",
            "d",
            "--apply",
            "--at",
            "0:64",
            "--drop-acknowledged",
        ];
        let command = parse(args(&full).into_iter()).unwrap();
        assert!(command.truncate && command.drop_acknowledged);
        assert_eq!(command.at, Some((0, 64)));
    }
}
