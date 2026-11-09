# segment-log

An embedded, append-only event log in Rust, for short-lived processes that need a durable history.

> **Status: early design. Nothing is built yet.**
> The repository holds the requirements, architecture and decision records. There is no working code, and both crates are placeholders. Some decision records are still proposed. The on-disk format is provisional until milestone M5, and the API can change. Do not depend on this yet. See the [milestones](docs/prd.md#milestones) for the plan.

- Many callers append to many streams in one directory.
- One writer thread packs their appends into groups. One `fsync` makes a group durable, and only then does the caller get an acknowledgment.
- Each stream has revisions and checks the caller's expected revision. An idempotency key makes retries safe after a crash.
- When the work ends, the log seals. The sealed segment files are the history.

![Without segment-log, every agent flushes its own file and the disk does one flush per agent. With segment-log, one writer packs many appends into one frame and one flush covers the group. Acks go out only after that flush returns.](docs/images/one-fsync-per-group.svg)

## Documents

- [Product requirements](docs/prd.md)
- [Architecture overview](docs/architecture/overview.md)
- [Byte layout v0](docs/architecture/format.md) · [API sketch](docs/architecture/api.md) · [Testing](docs/architecture/testing.md) · [Prior art](docs/architecture/prior-art.md)
- [Volume requirements](docs/architecture/volume-requirements.md): what the storage under the log must guarantee, and how to test it
- [Implement a behavior with acceptance tests](docs/acceptance-tests.md)
- [Glossary](docs/glossary.md)
- [Decisions](docs/decisions/README.md)

## Crates

| Crate                | Purpose                                                                                        |
| -------------------- | ---------------------------------------------------------------------------------------------- |
| `segment-log-core`   | The library.                                                                                   |
| `segment-log-server` | A Unix socket and vsock server for callers in other languages. Placeholder until milestone M6. |

## License

Apache-2.0. See [LICENSE](LICENSE).
