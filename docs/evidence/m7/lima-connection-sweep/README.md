# Lima sweep of the guest benchmark's connection count

This sweep picks the connection count C for the guest benchmark ([ADR 0063](../../../decisions/0063-the-guest-benchmark-shares-pipelined-connections.md)). It ran in the Lima rehearsal VM `fc` on the owner's Mac (nested KVM, Firecracker v1.18.0, 2 vCPUs, 1024 MiB), on 2026-11-17 at commit `887f690`. It is a rehearsal: it fills no row of the [qualification record](../qualification.md) and gives no bare-metal number.

Command, once for each C:

```sh
M7_PAIRS=1 M7_ROUNDS=300 M7_CONNECTIONS=$c scripts/m7-guest-bench.sh ~/sweep/c$c
```

| C | Guest closed loop, records/s | Guest paced at 50,000/s, achieved | Closed p50 ms |
| --- | --- | --- | --- |
| 1,000 (one per caller) | 6,725 | 7,975 | 66.7 |
| 1 | 20,820 | 19,725 | 45.0 |
| 4 | 30,595 | 32,835 | 28.4 |
| **16** | **44,624** | **43,068** | **17.9** |
| 64 | 41,802 | 40,565 | 19.7 |

The in-process side did 254,682 to 284,931 records/s in every run. Each `cN/` holds the run's `summary.md`, `environment.txt` and the guest's closed-loop thread sample.

## Result

- **C = 16**, the best rate of the sweep. `scripts/m7-guest-bench.sh` defaults to it, and the bare-metal run keeps it.
- At C = 16 the guest does 6.6× the rate with one connection per caller, which passes the relative pre-check before the bare-metal run.
- Below 16, `PIPELINE_DEPTH` (64 requests in flight on a connection) keeps some of the 1,000 callers from being in flight, so C = 1 and C = 4 are slow for that reason, not because of the transport.

## Limits

- In Lima, the run with one connection per caller is bound by the guest's vCPUs (`fc_vcpu` at 90–100 %, the `firecracker` thread at 10 %), not by the vsock muxer as on the qualified host. Nested virtualization makes guest exits expensive. So this sweep shows that the change cuts the cost per record, but not that it relieves the muxer. Only the bare-metal run can show that.
- The thread sample runs 10 s after the start. Runs at C ≥ 4 finished before it, so their samples show idle threads.
- One pair for each C, and the Mac runs other work. Read the differences between 16 and 64 as noise-sized.
