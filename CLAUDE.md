# segment-log

An embedded, append-only event log in Rust. One writer thread packs many callers' appends into groups, and one `fsync` makes a group durable. The log keeps per-stream revisions, checks expected revisions, removes duplicate retries by idempotency key, and seals when the work ends. Everything needed to build it is in this repository.

## Documents

Read these before you change code. Start with the PRD.

- [docs/prd.md](docs/prd.md): requirements (R1–R23), targets, milestones M0–M6, open items.
- [docs/architecture/overview.md](docs/architecture/overview.md): invariants I1–I10, write path, commit protocol, recovery, roll, seal.
- [docs/architecture/format.md](docs/architecture/format.md): byte layout v0. The only source for field widths.
- [docs/architecture/api.md](docs/architecture/api.md): public API and defaults. The only source for signatures.
- [docs/architecture/testing.md](docs/architecture/testing.md): crash model, oracle, benchmarks.
- [docs/acceptance-tests.md](docs/acceptance-tests.md): how to implement a behavior scenario-first, and where the tests go.
- [docs/architecture/prior-art.md](docs/architecture/prior-art.md): Kafka, the crate survey, and the evidence limits.
- [docs/glossary.md](docs/glossary.md): use each term with that meaning only.
- Decisions are recorded as ADRs (Architecture Decision Records) in `docs/decisions/`. Read [docs/decisions/README.md](docs/decisions/README.md) for the format and rules before writing or contradicting one.

All ADRs are `Proposed` until the owner accepts them. A Proposed ADR is still the working direction. Do not diverge from it without a new ADR.

## Rules

- **Invariants I1–I10 in [overview.md](docs/architecture/overview.md#invariants) must hold.** Never add an exception for one test or one path. If a design cannot satisfy them, change the design or write an ADR.
- **The milestone order is a gate.** Do not start a milestone before the "done when" check of the one before it passes. `segment-log-server` stays a placeholder until M6.
- **Write path:** one writer thread. No async runtime, no SQL, no protobuf. No `unsafe` without an ADR.
- **Dependencies:** `segment-log-core` uses only `std`. A new runtime dependency there needs an ADR ([0012](docs/decisions/0012-no-log-crate-dependency.md)). Dev-dependencies are fine.
- **File I/O** in the core goes through the storage trait, so that the crash tests can simulate it ([testing.md](docs/architecture/testing.md)).
- **Never retry an `fsync`** after it returns an error. Poison the writer ([0004](docs/decisions/0004-io-errors-poison-the-writer.md)). The retry on `EINTR` inside `std` does not count, because that call did not complete.
- **One source per fact.** Widths live in format.md, signatures and defaults in api.md, targets in the PRD. Other docs link to them. When code changes one of these facts, update that file in the same change.
- **Open items are open.** Do not settle a PRD open item in code. Propose an ADR.
- **Tests:** follow the [acceptance-test workflow](docs/acceptance-tests.md) for every implementation milestone, starting with M1. Each behavior starts with a Given/When/Then scenario that fails. Every change on the write or recovery path adds crash cases to the simulation. A bug fix starts with a failing test. A failing randomized test prints its seed and saves the storage trace needed to rebuild its crash state.

## Build and test

Toolchain: pinned in `rust-toolchain.toml`. Edition 2024.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI (`.github/workflows/ci.yml`) runs the same three commands on Linux and macOS.

## Writing

- Docs use short, active sentences, with one meaning for each word (as in the glossary).
- Comments say why, not what. A comment states a constraint or an invariant that the code cannot show.
- Keep a CHANGELOG once a release exists. Before the first release, there is none.
