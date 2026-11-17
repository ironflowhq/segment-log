# M6 socket diagnostic

The M6 row of the [milestone table](../../../prd.md#milestones) asks for a report in `docs/` that gives append throughput and latency over the Unix socket beside the same workload in process. These are diagnostics under [ADR 0029](../../../decisions/0029-m1-completion-requires-correctness-and-measurement-evidence.md), not a gate. The binding throughput and latency check is the guest benchmark in M7 ([ADR 0056](../../../decisions/0056-m6-builds-the-server-m7-validates-the-guest-deployment.md)).

Three files produce the report:

- `segment-log-server/examples/socket_bench.rs` runs the workload in process (`--inproc <dir>`) or over the Unix socket (`--unix <socket>`). Both sides run the same caller loop, so a difference between them is the transport: the encoding, the socket round trip and the server's two threads per connection.
- `scripts/m6-socket-report.sh` builds the release server and the bench, records the environment, and runs the sides alternately on fresh logs.
- `scripts/m6-socket-summary.py` writes `summary.md` and fails on a missing or unpaired run.

## The workload

The workload is the M1 benchmark's ([testing.md](../../../architecture/testing.md#benchmarks-m1-for-the-core-m6-for-the-server)), at the [api.md](../../../architecture/api.md) defaults. Each of 1,000 callers appends one keyed 256-byte record per round to its own stream, expecting the previous revision, for 1,000 rounds.

- **Closed loop:** each caller submits again as soon as it is answered. This run measures throughput.
- **Paced:** each caller submits once per 20 ms, which offers 50,000 records/s, the throughput target. Latency runs from the scheduled send ([ADR 0045](../../../decisions/0045-latency-target-is-measured-at-a-paced-rate.md)).

Over the socket each caller has its own connection and one request in flight. The server runs with `--max-connections 1000`. Every caller connects and says Hello before the clock starts.

Every run starts on a fresh log. A repeated key is a deduplicated retry that commits without a sync, so the bench fails on any deduplicated answer. Each mode has one unreported warm-up per side, then 5 pairs, in process first.

## Capture

Put the `m6-socket` label on a pull request, or dispatch the `m6-socket` workflow. It runs on a GitHub-hosted `ubuntu-26.04` runner, writes the logs under `/mnt`, and uploads the `m6-socket-linux` artifact for 3 days. On another host, run `scripts/m6-socket-report.sh OUT_DIR` from the repository root. `SEGMENT_LOG_EVIDENCE_DIR` puts the logs on the disk under test. `M6_SOCKET_SMALL=1` shrinks the sizes to check the tooling; never report its numbers. `M6_CALLERS`, `M6_ROUNDS`, `M6_PAIRS` and `M6_INTERVAL_US` set each size.

## Results

One capture, run [43677190849](https://github.com/ironflowx/segment-log/actions/runs/43677190849) at `65280cc`, on a hosted `ubuntu-26.04` runner: 4 vCPUs (AMD EPYC 9V45) and an ext4 root mounted `nobarrier`. [`linux/summary.md`](linux/summary.md) has the tables, and [`linux/environment.txt`](linux/environment.txt) has the full environment. Medians of 5 pairs, with the range in parentheses:

| Run | Metric | In process | Unix socket |
| --- | --- | --- | --- |
| Closed | records/s | 326,915 (310,572 to 329,891) | 73,636 (66,843 to 76,303) |
| Closed | p50 ms | 2.732 (2.694 to 2.859) | 12.581 (12.387 to 12.706) |
| Closed | p99 ms | 4.628 (4.372 to 5.326) | 30.471 (25.045 to 72.795) |
| Paced, 50,000/s offered | records/s | 49,970 (49,844 to 49,974) | 49,873 (49,820 to 49,898) |
| Paced, 50,000/s offered | p50 ms | 1.005 (0.999 to 1.040) | 1.437 (1.426 to 1.504) |
| Paced, 50,000/s offered | p99 ms | 30.106 (28.491 to 41.638) | 64.620 (60.947 to 71.627) |

No run was refused admission (`full=0` in every log), and every answer had the expected revision.

## Read the numbers with their limits

- **The disk does not bound these runs.** The root mount uses `nobarrier`, so `fdatasync` returns without a device flush. The in-process closed loop reaches 326,915 records/s, about 6.5 times the target, so the runner measures CPU and scheduling cost, not the storage. The device and its `fsync` latency are qualified in M7 ([ADR 0032](../../../decisions/0032-qualify-deployment-storage-before-m6.md)).
- **The socket side is CPU-bound on 4 vCPUs.** Over the socket, 1,000 bench threads and 2,000 server threads (two per connection, [ADR 0058](../../../decisions/0058-one-server-process-serves-one-log.md)) share 4 vCPUs, and the load average reached about 300. The closed loop's throughput falls by 77.5% and its p50 grows by about 10 ms. With the disk this fast, that gap is mostly scheduling: each record wakes a bench thread, the server's reader, the core's writer and the server's answer writer. On a disk whose `fsync` takes milliseconds, the same cost is a smaller share.
- **At the target rate, the socket keeps up.** The paced socket run delivers the offered 50,000 records/s. It adds 0.43 ms at p50 and about 35 ms at p99. Both sides have p99 tails of 30 ms or more on this shared runner. The socket p99 range does not overlap the in-process range, so the socket's added tail is measurable here, but its size depends on the runner.
- **The closed-loop tails vary between runs.** The socket's closed p99 ranges from 25 to 73 ms, and p999 from 91 to 297 ms. Read one capture as a sample, not as a target result.
- **Nothing here measures the guest.** The binding throughput and latency check, over vsock on the guest topology, is the M7 guest benchmark.

This report does not change the server. It gives a baseline for the cost of the two-threads-per-connection design under 1,000 connections, recorded as a diagnostic.
