# segment-log

An embedded, append-only event log in Rust, for short-lived processes that need a durable history.

- Many callers append to many streams in one directory.
- One writer thread packs their appends into groups. One `fsync` makes a group durable, and only then does the caller get an acknowledgment.
- Each stream has revisions and checks the caller's expected revision. An idempotency key makes retries safe after a crash.
- When the work ends, the log seals. The sealed segment files are the history.

![Without segment-log, every agent flushes its own file and the disk does one flush per agent. With segment-log, one writer packs many appends into one frame and one flush covers the group. Acks go out only after that flush returns.](docs/images/one-fsync-per-group.svg)

**Status: design only. Not usable yet.**

## Documents

- [Product requirements](docs/prd.md)
- [Architecture overview](docs/architecture/overview.md)
- [Byte layout v0](docs/architecture/format.md) · [API sketch](docs/architecture/api.md) · [Testing](docs/architecture/testing.md) · [Prior art](docs/architecture/prior-art.md)
- [Glossary](docs/glossary.md)
- [Decisions](docs/decisions/README.md)

## Crates

| Crate                | Purpose                                                                                        |
| -------------------- | ---------------------------------------------------------------------------------------------- |
| `segment-log-core`   | The library.                                                                                   |
| `segment-log-server` | A Unix socket and vsock server for callers in other languages. Placeholder until milestone M6. |

## License

Apache-2.0. See [LICENSE](LICENSE).
