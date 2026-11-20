# Changelog

Each release lists its API breaks with a migration note, its format version bumps and the protocol versions it speaks ([ADR 0064](docs/decisions/0064-the-0-1-compatibility-promise.md)). The crates and the Go client (`clients/go/vX.Y.Z`) share one version.

## 0.1.1 - 2026-11-20

- **Go client:** `Pool` (`DialPool(network, address, n)`) shares *n* pipelined connections among goroutines. Requests queued on a connection while a write is in progress go out together in the next write, and each answer returns to its caller in request order. A connection holds no more requests in flight than the server reads ahead ([protocol.md](docs/architecture/protocol.md), step 3); a request beyond that waits for an answer. When a connection fails, each request written to it and not answered gets `ErrUnknown` and is never retried by the pool; requests not yet written get `ErrNotSent`. The next request for that connection dials a new one, and gets `ErrNotSent` if that fails. After `Close`, requests get `ErrPoolClosed`. The pool refuses `Hello` ([issue 134](https://github.com/ironflowx/segment-log/issues/134), [ADR 0063](docs/decisions/0063-the-guest-benchmark-shares-pipelined-connections.md)). Additive; no wire change.
- **Speaks protocol versions:** 2.

### Fixed

- **Open no longer drops acknowledged records** when a bad frame in the newest unsealed segment has valid group frames of the log after it ([issue 178](https://github.com/ironflowx/segment-log/issues/178), [ADR 0068](docs/decisions/0068-open-refuses-damage-with-acknowledged-groups-after-it.md)). Open used to truncate at the bad frame and return `Ok`, so the next writer reused those offsets and a retry of a dropped key ran twice. It now fails with `OpenError::Corrupt { segment, position }` at the bad frame and changes no file. A torn tail is still truncated. The same damage in a second-newest segment without its closing run now fails with `Corrupt` at the bad frame instead of `Gap` at the next segment, and `LogReader` on such a log returns `Corrupt` instead of `NotSealed`. No API change.
- **`segment-log-server repair` gains class F** for that case. `truncate` plans it as lossy: `--apply --drop-acknowledged` writes a seal run at the bad frame and leaves the log sealed, and refuses when the run would reach the last acknowledged group frame, or while the second-newest segment lacks its closing run. Class F in a second-newest segment is diagnosed only. The JSON `class` field can now be `"F"`; `schema` stays 1.

No format version change. The protocol versions are unchanged.

## 0.1.0 - 2026-11-18

The first production release. From this release, every later build reads every format version from 3 and speaks every protocol version from 2; no 0.x release removes either. API breaks go only in minor releases.

- **Format version 3**, the baseline. Each group carries a u64 append time that never decreases within a log. Logs written before 0.1 (format versions 0, 1 and 2) are refused with `UnsupportedVersion`.
- **Protocol version 2**, the baseline. `Hello` negotiates a version range. Clients written before 0.1 (protocol version 1) are refused with `UnsupportedVersion`.
- **Speaks protocol versions:** 2.
- **License:** MIT, for the crates and the Go client. The `0.0.1` placeholder crates on crates.io stay Apache-2.0.
- **`segment-log-core`:** append with group commit (one `fsync` per group), per-stream revisions and expected-revision checks, idempotency keys, roll, seal, keyed lookup through each sealed segment's key filter and index, `LogReader` for sealed history, and crash recovery ([api.md](docs/architecture/api.md)).
- **`segment-log-server`:** one process serves one log over a Unix socket or vsock; `--read-only` serves a sealed log; `repair verify|truncate` for a log that opens as `Corrupt` ([server README](segment-log-server/README.md), [ADR 0065](docs/decisions/0065-the-repair-tool-truncates-only-the-newest-segment.md)).
- **Writer tail latency** ([issue 154](https://github.com/ironflowx/segment-log/issues/154)): the active key map is a hash map, sized at each roll so it never grows on the writer (#163, #168); the log filter is built off the writer (#164); the sealer thread writes and flushes a rolled segment's closing run, so appends no longer wait for it ([ADR 0067](docs/decisions/0067-the-sealer-writes-and-flushes-the-closing-run.md), #170). Measurements are in the [issue 154 record](docs/evidence/issue154/README.md).
- **Go client** `github.com/ironflowx/segment-log/clients/go`: appends and reads over either transport.
- **Qualified stack:** Firecracker v1.18.0 on a dedicated local-NVMe host passes every MUST volume requirement ([qualification record](docs/evidence/m7/qualification.md)). Any other stack needs its own qualification.

### Known limits

- In a Firecracker guest, append throughput reaches 52,860 records/s, but the paced p99 at 50,000 records/s is 69 ms, above the latency target ([ADR 0063](docs/decisions/0063-the-guest-benchmark-shares-pipelined-connections.md)).
- On an AMD EPYC 7401P host, the in-process paced p99 at 50,000 records/s was 29 ms against a 1.94 ms bound, measured before the writer tail-latency fixes above. On an EPYC 7502P after #168, but before ADR 0067, it was 4.1 ms against a 1.8 ms bound. An EPYC rerun with ADR 0067 is pending ([issue 154](https://github.com/ironflowx/segment-log/issues/154)).
- A Go `Client` is one connection that goroutines cannot share; there is no pooled client yet ([issue 134](https://github.com/ironflowx/segment-log/issues/134)).
- No replication, retention or compaction ([PRD non-goals](docs/prd.md#non-goals)).
