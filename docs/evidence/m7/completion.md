# M7 completion record

M7 (validate the server on the guest deployment) passed on 2026-11-18. This record links the evidence for each part of the [M7 gate](../../prd.md#milestones). Every part comes from one session on one host, Hetzner auction host #3115170 (AMD EPYC 7401P, two Samsung PM983 drives), on 2026-11-17 and 2026-11-18, with `scripts/m7-session.sh` at `386695f`, format version 3 and protocol version 2 ([requalified-host/](requalified-host/README.md), [PR 155](https://github.com/ironflowx/segment-log/pull/155)). The M7 decisions, [ADR 0059](../../decisions/0059-m7-qualifies-firecracker-on-a-dedicated-nvme-host.md), [ADR 0060](../../decisions/0060-the-server-reads-a-sealed-log-with-read-only.md), [ADR 0062](../../decisions/0062-v3-requires-refused-writes-not-failed-syncs.md) and [ADR 0063](../../decisions/0063-the-guest-benchmark-shares-pipelined-connections.md), are accepted.

| Gate part | Evidence |
| --- | --- |
| The qualification record for the actual guest stack passes every MUST requirement. | The [qualification record](qualification.md): status qualified. Every MUST row of V1–V11 is `Pass`, and V8 is `Not applicable` (the deployment never copies a log volume). The power cuts include 30 host power losses through the Robot `power_long` reset; both drives' SMART `power_cycles` went from 16 to 47, one per cut. Every known-bad control caught its loss. The [fingerprint](../../../scripts/m7/fingerprint) is committed, and `VOL-DRIFT` refuses a change to each checked key. |
| The handoff record identifies mount paths, the reader component and the procedure, and `HANDOFF` passes. | The [handoff record](handoff.md) names the volume, the writer and reader mount paths, the reader (`segment-log-server --read-only`) and the procedure. `TestHandoffSealed` and `TestHandoffUnsealed` pass over Firecracker vsock in [rehearse.log](requalified-host/rehearse.log) (qualification row V6). |
| The shared conformance cases pass over vsock on the selected guest topology. | [rehearse.log](requalified-host/rehearse.log), "conformance over Firecracker vsock": `TestConformance`, `TestTransportUnknown`, `TestTransportRestart`, `TestDialBurst` and `TestThreadExhaustion` pass, with the server in a Firecracker guest and the Go client on the host over hybrid vsock. |
| The guest benchmark report is in `docs/`, measures the throughput and latency targets and names the qualified stack. | The [guest benchmark summary](requalified-host/guest-bench/summary.md) and the qualification record's [guest benchmark section](qualification.md#guest-benchmark-and-49), which names the qualified stack by its fingerprint. 5 pairs of guest and in-process runs, closed loop and paced at 50,000 records/s. |
| One guest meets the throughput target, with its callers sharing the pipelined connections that ADR 0063 fixes. | Closed loop, 1,000 callers over 16 shared connections: **52,860 records/s** median (51,631 to 53,338). Every pair is above the 50,000 records/s target. |

## Known limits

These are measured targets, not gate parts. Each is recorded for 0.1.

- **Guest latency at the target rate.** Paced at 50,000 records/s, the guest's append p99 is 69.2 ms against the PRD's latency target. The guest runs at about 94 % of its closed-loop rate there, so the tail is queueing near saturation, bound by Firecracker's vsock muxer thread. ADR 0063 records this as a known 0.1 limit.
- **The core's paced tail on this CPU.** In process on the same host, paced at 50,000 records/s, append p99 is 29.1 ms median against a 1.94 ms bound, in 0 of 5 runs. The first qualified host (Xeon E-2176G) met the bound in 5 of 5. Rolls and device flushes are not the cause ([issue 154](https://github.com/ironflowx/segment-log/issues/154)).
- **No connection pooling in the Go client.** The benchmark shares connections in its own harness; the client does not yet ([issue 134](https://github.com/ironflowx/segment-log/issues/134)).
- **Provisional values.** The V7 failure domain, the V8 statement, the V9 workload profile and the V11 density are the owner's provisional values ([qualification record](qualification.md#provisional-values)). A change to any of them needs a new qualification row, not a new session, unless the fingerprint changes.

## Earlier sessions

Stage 1 ran on a [dedicated host](dedicated-host/). The [first qualified host](qualified-host/README.md) offered no power button reset, and the [power-loss host](power-loss-host/README.md) was a second machine. ADR 0059's amendment requires every row from one host, so neither fills a row of the record.
