//! The command line in ADR 0058. Every `Options` field has a flag; omitted flags keep api.md's defaults.

use segment_log_core::Options;
use std::path::PathBuf;
use std::time::Duration;

pub const USAGE: &str = "usage: segment-log-server --dir <path> (--unix <socket-path> | \
--vsock-port <port>) [--linger-us <n>] [--group-byte-cap <n>] [--roll-bytes <n>] [--queue-capacity <n>] \
[--pending-byte-budget <n>] [--max-connections <n>]";

/// Where the server listens: exactly one transport per process (ADR 0058).
pub enum Listen {
    Unix(PathBuf),
    Vsock(u32),
}

pub struct Config {
    pub dir: PathBuf,
    pub listen: Listen,
    pub options: Options,
    pub max_connections: usize,
}

pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Config, String> {
    let (mut dir, mut unix, mut vsock) = (None, None, None);
    let mut options = Options::default();
    let mut max_connections = 64;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let number = || {
            value
                .parse::<u64>()
                .map_err(|_| format!("{flag}: not a number: {value}"))
        };
        let size = || number().map(|n| n as usize);
        match flag.as_str() {
            "--dir" => dir = Some(PathBuf::from(&value)),
            "--unix" => unix = Some(PathBuf::from(&value)),
            "--vsock-port" => {
                vsock = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("{flag}: not a port: {value}"))?,
                )
            }
            "--linger-us" => options.linger = Duration::from_micros(number()?),
            "--group-byte-cap" => options.group_byte_cap = size()?,
            "--roll-bytes" => options.roll_bytes = number()?,
            "--queue-capacity" => options.queue_capacity = size()?,
            "--pending-byte-budget" => options.pending_byte_budget = size()?,
            "--max-connections" => max_connections = size()?,
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    let listen = match (unix, vsock) {
        (Some(path), None) => Listen::Unix(path),
        (None, Some(port)) => Listen::Vsock(port),
        _ => return Err("give exactly one of --unix or --vsock-port".into()),
    };
    Ok(Config {
        dir: dir.ok_or("--dir is required")?,
        listen,
        options,
        max_connections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn flags_override_the_defaults_and_the_rest_keep_them() {
        let config = parse(args(&[
            "--dir",
            "d",
            "--unix",
            "s",
            "--linger-us",
            "250",
            "--group-byte-cap",
            "4096",
        ]))
        .unwrap();
        assert_eq!(config.options.linger, Duration::from_micros(250));
        assert_eq!(config.options.group_byte_cap, 4096);
        assert_eq!(
            config.options.queue_capacity,
            Options::default().queue_capacity
        );
        assert_eq!(config.max_connections, 64);
    }

    #[test]
    fn missing_or_unknown_flags_are_errors() {
        assert!(parse(args(&["--unix", "s"])).is_err());
        assert!(parse(args(&["--dir", "d"])).is_err());
        assert!(parse(args(&["--dir", "d", "--unix", "s", "--bogus", "1"])).is_err());
        assert!(parse(args(&["--dir", "d", "--unix", "s", "--roll-bytes", "x"])).is_err());
        assert!(parse(args(&["--dir"])).is_err());
    }

    #[test]
    fn exactly_one_transport() {
        let config = parse(args(&["--dir", "d", "--vsock-port", "5000"])).unwrap();
        assert!(matches!(config.listen, Listen::Vsock(5000)));
        assert!(parse(args(&["--dir", "d", "--unix", "s", "--vsock-port", "1"])).is_err());
        assert!(parse(args(&["--dir", "d", "--vsock-port", "-1"])).is_err());
    }
}
