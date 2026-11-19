# Evidence

One row per investigation. Each record states its question, host, date, the commits it measured, and the numbers it relies on. Update a row when a later run changes its verdict ([CLAUDE.md](../../CLAUDE.md#evidence)).

Raw run output (logs, traces, TSVs) is not kept in this repository, nor in its history. The raw files of the records written before 2026-11-19 are frozen in [segment-log-evidence](https://github.com/ironflowx/segment-log-evidence/tree/dc1a230a4b0e7bd70fec1963473ddd7cb2733c1c), under the same paths as here, and the records link to them there. A path a record names in plain text, without a link, is a path in that archive; its directory names keep the commit hashes from before the history rewrite of 2026-11-19.

## Milestones

| Record | Question | Host | Date | Verdict |
| --- | --- | --- | --- | --- |
| [m1](m1/README.md) | Keep the M1 slice PR evidence and its provenance limits | Mac; GitHub runner | — | Archive of slices 0–9. Some early red outputs are missing. It does not establish M1 completion. |
| [m1/completion](m1/completion.md) | Evidence for each M1 completion gate item on the final tree | GitHub runner; Docker on Mac; Mac | 2026-11-10 | All four gate items pass. 198,198 and 424,813 records/s. Paced p99 within bound in 4 of 5 runs (Docker), 0 of 5 (hosted). |
| [m1/latency](m1/latency/README.md) | Why append p99 misses its bound, and whether a CRC byte table helps | Hetzner CCX23 VM | — | CRC table cut p50 16.551 → 15.558 ms and reopen 2,048 → 1,366 ms. p99 not improved. Every run misses the bound. |
| [m1/slice9](m1/slice9.md) | Threaded trace oracle and M1 coverage audit | Mac | — | 2,700 scenarios over 100 seeds pass. Seed 1000 found a harness bug, now fixed. No production defect. |
| [m1/slice10](m1/slice10.md) | Earlier M1 completion evidence, before the issue 20 changes | Docker on Mac; Mac | — | Syscall and directory checks pass. About 150k records/s. Every run misses the latency bound. The completion record replaces it. |
| [m3/completion](m3/completion.md) | Evidence for the M3 segment roll gate | GitHub runner | 2026-11-10 | Passed. Roll crash tests are green. strace shows fdatasync and directory fsync before the first group frame in the new segment. |
| [m4/completion](m4/completion.md) | Evidence for the M4 gate: sealed open, key lookup, Linux report | GitHub runner | 2026-11-11 | Passed. Keyed warm open p50 38.6 ms (target 50 ms). Keyed appends 109,100 records/s, about half of M3. |
| [m4](m4/README.md) | How the M4 Linux report is captured and how to read it | GitHub runner | — | Method only, no results. Says no full capture has run yet. |
| [m4/key-lookup](m4/key-lookup/README.md) | Which retained-key lookup design ADR 0048 should choose | Mac | — | Diagnostics only. A device read on every new key (about 76 µs) cannot meet the append target. |
| [m5/completion](m5/completion.md) | Evidence for the M5 gate: frozen format, 1-hour fuzzing | GitHub runner | 2026-11-13 | Passed. ADR 0052 accepted. Both fuzz targets ran 3,601 s with no crash. Known limit: recovery does not check a footer against its groups. |
| [m6/completion](m6/completion.md) | Evidence for the M6 server gate over Unix socket and vsock | GitHub runner | 2026-11-15 | Passed. Conformance passes on both transports. 1-hour protocol fuzz has no crash. Binary is 1,049,384 bytes. Idle RSS is 1,240 KiB. |
| [m6/socket](m6/socket/README.md) | Append throughput and latency over the Unix socket versus in process | GitHub runner | — | Paced socket keeps 49,873 records/s, p99 64.6 vs 30.1 ms. Closed loop is CPU-bound: 73,636 vs 326,915 records/s. |
| [m7/completion](m7/completion.md) | Evidence for the M7 gate: the server on the guest deployment | EPYC 7401P, PM983 | 2026-11-17 | Passed. Stack qualified. Guest closed loop 52,860 records/s. Guest paced p99 69.2 ms misses the latency target. |
| [m7/qualification](m7/qualification.md) | Does the ADR 0059 stack pass volume requirements V1–V11? | EPYC 7401P, PM983 | 2026-11-17 | Qualified. Every MUST row passes and V8 is not applicable. 0 of 30 host power cuts lost a record. |
| [m7/handoff](m7/handoff.md) | Mount paths, reader component and the writer-to-reader procedure | Lima VM; i7-7700, PM981 | 2026-11-16 | TestHandoffSealed and TestHandoffUnsealed pass in Lima and on the dedicated host. |
| [m7/dedicated-host](m7/dedicated-host/README.md) | First bare-metal rehearsal and guest-side power cuts | i7-7700, PM981 | — | Rehearsal passes. 0 of 30 cycles lost a record for each guest event. Controls caught 10 of 10. Fills no row. |
| [m7/power-loss-host](m7/power-loss-host/README.md) | Host power loss, and the guest benchmark before and after ADR 0063 | EPYC 7401P, PM963 | 2026-11-17 | power_long cut drive power: power_cycles 54 → 85, one per cut. Second machine, so it fills no row. |
| [m7/qualified-host](m7/qualified-host/README.md) | Stage 2 qualification session on the first qualified host | Xeon E-2176G, PM983 | 2026-11-17 | No power button reset, so host power loss could not run. The hw reset kept the drives powered (power_cycles 21). |
| [m7/requalified-host](m7/requalified-host/README.md) | Raw output of the one-host requalification session | EPYC 7401P, PM983 | 2026-11-17 | Every step ran on one host. 30 cuts removed drive power (power_cycles 16 → 47). v2-clean reran after an ssh timeout. |
| [m7/lima-connection-sweep](m7/lima-connection-sweep/README.md) | Pick the guest benchmark's connection count | Lima VM | 2026-11-17 | C = 16 is best: 44,624 records/s, 6.6× one connection per caller. Rehearsal only. |
| [m7/lima-v3-v9](m7/lima-v3-v9/README.md) | Rehearse the V3 metadata-error and V9 ENOSPC cases on the M7 guest | Lima VM | 2026-11-16 | V3 fails under every errors= mode because fsync succeeds after the journal aborts. V9 passes: every acknowledged record survives. |

## Issue investigations

| Record | Question | Host | Date | Verdict |
| --- | --- | --- | --- | --- |
| [issue20/hosted](issue20/hosted.md) | Simulator warnings, hosted long-run execution and hardware gaps | Mac; GitHub runner | 2026-11-09 | Gate fix removes 24 warnings. Manual long run passed on Linux and macOS. Scheduled run and failure upload not proven. |
| [issue20/ack-relay](issue20/ack-relay/README.md) | Where the 1,000-caller group cycle goes, and whether the ack relay helps | Docker on Mac | 2026-11-09 | Delivery 3.23 → 0.26 ms. 167,928 → 323,552 records/s. p99 7.07 → 5.47 ms. Bound still missed. |
| [issue20/combined](issue20/combined/README.md) | The six issue 20 changes together, against the earlier main | Docker on Mac | 2026-11-09 | Closed loop 156,239 → 414,634 records/s. Paced p99 3.871 → 2.090 ms. Bound missed in every run. |
| [issue20/concurrent-macos](issue20/concurrent-macos/README.md) | Does a concurrent reader change append throughput on a growing history? | Mac | 2026-11-09 | Inconclusive: about 0.5% difference, within run ranges. Profiles show the reader blocked on the storage mutex. |
| [issue20/crc-slicing](issue20/crc-slicing/README.md) | Does an eight-byte-step CRC cut encode, reopen and read cost? | Docker on Mac | 2026-11-09 | Encode 0.65 → 0.21 ms. Reopen 861–919 → 476–567 ms. Read p50 17 → 5 µs. p99 gain not established. |
| [issue20/crc-tables](issue20/crc-tables/README.md) | Why hosted CI aborted in a debug-build codec scenario | GitHub runner; Mac; Docker on Mac | — | A `const` table was copied on every lookup. A `static` cut the scenario about 50×. Optimized code is unchanged. |
| [issue20/hosted-drill](issue20/hosted-drill/README.md) | Does a failed long run upload artifacts that replay? | GitHub runner; Mac | — | Yes. Planted failures on Linux and macOS uploaded artifacts. Replays passed in CI and on a Mac. |
| [issue20/key-positions](issue20/key-positions/README.md) | Memory and append effect of keeping only each key's offset and revision | Docker on Mac | 2026-11-09 | 179.4 → 81.9 bytes per key, −54% at 1M keys. 168,148 → 176,304 records/s. Bound still missed. |
| [issue20/linger-admission](issue20/linger-admission/README.md) | Effect of counting linger from the first submit's admission (ADR 0046) | GitHub runner; Docker on Mac | — | Paced p50 fell in all 20 pairs (1.52 → 1.17 ms hosted). p99 within bound in 3 of 20 runs after, 1 of 20 before. |
| [issue20/linux-phase-peaks](issue20/linux-phase-peaks/README.md) | Steady-state and peak resident memory per phase, from kernel counters | Docker on Mac | 2026-11-10 | Each phase peaks at its own steady state, as a lower bound. Empty engine 316 KiB. The kernel mark reads up to 576 KiB low. |
| [issue20/mac-p99](issue20/mac-p99/README.md) | Where the slowest 1% of requests wait, from request timelines | Mac | — | Group formation plus flush is 93.9–96.0% of tail latency at 64 callers. Observer overhead inconclusive. |
| [issue20/mac-resident](issue20/mac-resident/README.md) | Resident memory isolated by phase and history dimension | Mac | 2026-11-09 | Phases isolated. 16,384 keys add 3,776 KiB RSS. Exact engine-owned bytes and most short peaks inconclusive. |
| [issue20/paced-benchmark](issue20/paced-benchmark/README.md) | A paced 50,000 records/s variant, to separate log latency from the closed loop | Docker on Mac | 2026-11-09 | All 10 paced runs miss the bound. Unstalled runs: p99 2.6–3.9 ms against about 2 ms. Device stalls set the rest. |
| [issue20/positioned-reads](issue20/positioned-reads/README.md) | Do positioned reads stop readers waiting on group flushes? | Docker on Mac | 2026-11-09 | Yes. Read p99 238–393 → 22–24 µs. Slow reads per run 1,024 → 0–1. Appends unchanged. |
| [issue20/reader-crash-cancel](issue20/reader-crash-cancel/README.md) | Cause of the long-run failure at seed 10030 | Docker on Mac; Mac | — | Harness bug: the reader panicked on a read that a crash canceled. The saved pair replays clean. The engine was correct. |
| [issue20/stream-heads](issue20/stream-heads/README.md) | Does a group's cost stop growing with the number of streams? | Docker on Mac | 2026-11-09 | Yes. 2,000 groups at 100,000 streams: 7.4–11.5 → 0.60–0.73 s. No established change in the default benchmark. |
| [issue47](issue47/README.md) | Throughput, reopen and memory at larger and longer workloads | EPYC 7401P, PM983 | — | 0 correctness errors. 120,000–130,000 records/s up to 4,000 callers. 60M records reopen in 219 ms. About 34 bytes retained per record. |
| [issue48](issue48/README.md) | Exact phase peaks and engine-owned bytes, with how exact each figure is | Docker on Mac | 2026-11-10 | Every phase peaks at the larger of its two ends (exact). Empty engine requests 289.7 KiB. About 78–82 bytes per key. |
| [issue49](issue49/README.md) | M1 capture on bare-metal local NVMe, main versus the M1 tree | i7-7700, PM981 | 2026-11-16 | Throughput met: 62,203 records/s. Paced p99 on main 83.13 ms, 0 of 5 within bound, from roll stalls (issue 107). |
| [issue70](issue70/README.md) | How key lookup cost grows with sealed history, before and after ADR 0051 | GitHub runner | — | 4.65 → 30.86 µs per record from 100 to 1,000 segments (29,105 records/s). The log key filter holds it at 1.2–2.5 µs. |
| [issue154](issue154/README.md) | Why paced p99 misses the bound on the EPYC host | EPYC 7502P, PM983 and PM9A3; Ryzen 9 3900 | 2026-11-18 | Key map sizing (#168) cut paced p99 8.95 → 4.14 ms on EPYC; 1.8 ms bound still missed. ADR 0067 (#170) cut p999 23.3 → 15.2 ms on Ryzen, where main already passes. Not yet rerun on EPYC. |
| [issue156](issue156/README.md) | Several readers beside several appenders on a large history, and resident memory per stream | Apple M4 Mac mini, internal SSD | 2026-11-19 | Diagnostics. Up to 4 readers show no cost to 1,000 appenders; 16 history readers raise append p99 to 33–38 ms. Random reads past 16 sealed segments take 1.5–6 ms, because each rereads a segment's frame headers. About 230 bytes per stream. |

## Other

| Record | Question | Host | Date | Verdict |
| --- | --- | --- | --- | --- |
| [format/key-page-alignment](format/key-page-alignment/README.md) | Cost of a cold key index page read, aligned versus unaligned | GitHub runner | — | Unaligned pages take the same device reads but twice the bytes. p50 about 3% (4 µs) higher. p99 within the spread. |
| [m1-growth](m1-growth/README.md) | How memory, reads and reopen grow with history size on Linux | Hetzner CCX23 VM | 2026-11-09 | 90 runs pass. Retained heap grows with keys, streams and groups, not with records. No leak established. |
| [m1-memory](m1-memory/README.md) | Cut a one-record read's allocation by bounding the frame snapshot | Mac | 2026-11-09 | Start read 21,207 → 767 bytes, end read 21,143 → 703 bytes, the same in all 5 runs. All 120 runs validated. |
