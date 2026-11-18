# Changelog

Each release lists its API breaks with a migration note, its format version bumps and the protocol versions it speaks ([ADR 0064](docs/decisions/0064-the-0-1-compatibility-promise.md)). The crates and the Go client (`clients/go/vX.Y.Z`) share one version.

## 0.1.0 - 2026-11-18

The first production release. From this release, every later build reads every format version from 3 and speaks every protocol version from 2; no 0.x release removes either. API breaks go only in minor releases.

- **Format version 3**, the baseline. Each group carries a u64 append time that never decreases within a log. Logs written before 0.1 (format versions 0, 1 and 2) are refused with `UnsupportedVersion`.
- **Protocol version 2**, the baseline. `Hello` negotiates a version range. Clients written before 0.1 (protocol version 1) are refused with `UnsupportedVersion`.
- **Speaks protocol versions:** 2.
- **License:** MIT, for the crates and the Go client. The `0.0.1` placeholder crates on crates.io stay Apache-2.0.
- **`segment-log-core`:** append with group commit (one `fsync` per group), per-stream revisions and expected-revision checks, idempotency keys, roll, seal, keyed lookup through each sealed segment's key filter and index, `LogReader` for sealed history, and crash recovery ([api.md](docs/architecture/api.md)).
- **`segment-log-server`:** one process serves one log over a Unix socket or vsock; `--read-only` serves a sealed log; `repair verify|truncate` for a log that opens as `Corrupt` ([server README](segment-log-server/README.md), [ADR 0065](docs/decisions/0065-the-repair-tool-truncates-only-the-newest-segment.md)).
- **Go client** `github.com/ironflowx/segment-log/clients/go`: appends and reads over either transport.
- **Qualified stack:** Firecracker v1.18.0 on a dedicated local-NVMe host passes every MUST volume requirement ([qualification record](docs/evidence/m7/qualification.md)). Any other stack needs its own qualification.

### Known limits

- In a Firecracker guest, append throughput reaches 52,860 records/s, but the paced p99 at 50,000 records/s is 69 ms, above the latency target ([ADR 0063](docs/decisions/0063-the-guest-benchmark-shares-pipelined-connections.md)).
- On an AMD EPYC 7401P host, the in-process paced p99 at 50,000 records/s is 29 ms against a 1.94 ms bound ([issue 154](https://github.com/ironflowx/segment-log/issues/154)).
- A Go `Client` is one connection that goroutines cannot share; there is no pooled client yet ([issue 134](https://github.com/ironflowx/segment-log/issues/134)).
- No replication, retention or compaction ([PRD non-goals](docs/prd.md#non-goals)).
