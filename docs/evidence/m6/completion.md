# M6 completion record

M6 (`segment-log-server` over a Unix socket, then vsock with the same protocol) passed on 2026-11-15. This record links the evidence for each part of the [M6 gate](../../prd.md#milestones). The M6 decisions, [ADR 0056](../../decisions/0056-m6-builds-the-server-m7-validates-the-guest-deployment.md), [ADR 0057](../../decisions/0057-a-hand-rolled-binary-protocol-answered-in-order.md) and [ADR 0058](../../decisions/0058-one-server-process-serves-one-log.md), are still Proposed. The gate does not require their acceptance.

All CI evidence below is from [CI run 43677799986](https://github.com/ironflowhq/segment-log/actions/runs/43677799986) at `6019cb5` on `main`, unless a row names another run.

| Gate part | Evidence |
| --- | --- |
| A Go test client appends and reads over a Unix socket and over vsock through the Linux host loopback. | `clients/go` runs over both transports. The `conformance` job passes over the Unix socket on ubuntu-26.04 and macOS, and over vsock on ubuntu-26.04 after `sudo modprobe vsock_loopback`. |
| `TRANSPORT-CONFORMANCE`, `TRANSPORT-UNKNOWN` and `TRANSPORT-RESTART` pass over both, from shared conformance cases for retries, revisions and admission outcomes. | All three `conformance` entries pass `TestConformance` (the 9 cases in `conformance/`), `TestTransportUnknown`, `TestTransportRestart`, and the poison, seal and signal exits of ADR 0058. The cases are data, so both transports run the same files ([testing.md](../../architecture/testing.md#m6-transport-order-and-conformance)). |
| `FUZZ-PROTOCOL` runs for 1 hour in CI with no crash. | [Fuzz run 43677821633](https://github.com/ironflowhq/segment-log/actions/runs/43677821633), dispatched at `6019cb5` on hosted ubuntu-26.04 runners with `nightly-2026-11-12`, starting from the seeds alone. `FUZZ-PROTOCOL` ran for 3,601 s with no crash: 724,321,539 runs, reaching 265 coverage points and a 94-entry corpus. The other three targets passed their hour in the same run. |
| A Linux CI job meets the idle resident set and binary size targets. | The `footprint` job, an x86_64 musl release build on ubuntu-26.04: the stripped binary is 1,049,384 bytes (target at most 4 MiB), and the idle resident set on an empty log after open is 1,240 KiB (target at most 8 MiB). `scripts/m6-footprint.sh` measures both. |
| A report in `docs/` gives append throughput and latency over the Unix socket beside the same workload in process, as diagnostics under ADR 0029. | The [socket report](socket/README.md), from [capture 43677190849](https://github.com/ironflowhq/segment-log/actions/runs/43677190849). At the 50,000 records/s target rate, the socket delivered 49,873 records/s with p50 1.4 ms and p99 64.6 ms; in process, 49,970 records/s with p50 1.0 ms and p99 30.1 ms. These are diagnostics. The runner's disk does not flush, and the binding check is the M7 guest benchmark. |

## How to reproduce each path

testing.md asks for the protocol version, commands, topology and results of each path.

- **Protocol:** version 1, as [protocol.md](../../architecture/protocol.md) defines it. Every connection opens with Hello (`SGLP`, version 1).
- **Server:** `cargo build -p segment-log-server`. The tests start `segment-log-server --dir <dir> --unix <socket>`, or `--vsock-port <port>` for vsock, and add case-specific flags such as `--linger-us`.
- **Client:** from `clients/go`, `SEGMENT_LOG_SERVER=<binary> go test -count=1 -v ./...`. Add `SEGMENT_LOG_TRANSPORT=vsock` for vsock. CI used go1.27.4, the newest 1.27 release that `actions/setup-go` resolves from `go.mod`.
- **Topology:**
  - Unix: the server and client on one host, through a socket file under `/tmp`.
  - vsock: one Linux host. The server listens on any CID, and the client dials CID 1 (`VMADDR_CID_LOCAL`) through `vsock_loopback`.
- **Results:** every Go test passed on every entry (job logs [vsock](https://github.com/ironflowhq/segment-log/actions/runs/43677799986/job/132701661951), [Unix on Linux](https://github.com/ironflowhq/segment-log/actions/runs/43677799986/job/132701662075), [Unix on macOS](https://github.com/ironflowhq/segment-log/actions/runs/43677799986/job/132701662124)).

## Known limits

- **vsock loopback is not the guest topology.** It shows the adapter and the protocol over `AF_VSOCK`, not connectivity from a guest. That evidence belongs to M7 ([ADR 0056](../../decisions/0056-m6-builds-the-server-m7-validates-the-guest-deployment.md)).
- **Two threads per connection.** At 1,000 connections on 4 vCPUs, the socket closed loop was CPU-bound at 73,636 records/s, against 326,915 in process ([socket report](socket/README.md)). This is a recorded diagnostic, not a gate miss.
- **Status codes no socket test reaches.** No test over a socket produces `Full`, `Exhausted`, `Poisoned`, `Corrupt` or `Io`, and only the poison test produces `Indeterminate`. The protocol tests in the server crate pin the encoding of each.
