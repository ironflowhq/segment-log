# Linux memory, read, and reopen growth diagnostics

The goal is to complete the retained-history evidence for issue [#20](https://github.com/ironflowhq/segment-log/issues/20) after [PR #22](https://github.com/ironflowhq/segment-log/pull/22). PASS for the bounded diagnostic run and validation. This evidence does not qualify M1 for production or close the issue.

## Source and scope

Production baseline is `911e809e72fc9c9ef5571f7022c89e55a48d6cb1`. The measured library is PR22 head `08aee0d8cdeaf0aa029e8dab56d4536fb50771de`. Diagnostic source is `531843ea02c1b8d6dd54cea2bb980e250e946633`, whose library is identical to PR22. No CRC candidate, format, default, API, or production implementation changes are included. This PR depends on PR22's bounded read snapshot and allocator probe design and is stacked on `t3/read-recovery-memory`.

The Rust diagnostic executable was built from the byte-identical example at `896f599a88904ba75c45e640902b85ef3f0dae5e`. The Python runner gained two fixed-group cases in `531843e`; it did not rebuild or change that executable. [source-hashes.txt](source-hashes.txt) records matching remote and local source hashes. The release binary SHA-256 is `14865accd6ba4a51491b34ac11d55b174a639cbe4bed595c7d03c8fd5c391128`.

Captured November 9, 2026, on an exclusive Hetzner CCX23 guest with four vCPUs and 16 GiB configured RAM. Ubuntu 24.04, Linux 6.8.0-138-generic, KVM, x86_64, AMD EPYC-Milan virtual CPU, glibc 2.39, Rust 1.99.1, compiler `58cf8f9c35b355199727627d07ae34a4e2846d67`, LLVM 23.1.1. Data and executable lived under `/root/segment-log-memory` on `/dev/sda1`, ext4, approximately 160 GB virtual disk. [environment.json](environment.json) preserves CPU, memory, filesystem, process list, compiler, and binary hash. The latency worker had released the host before any build or measurement. `/root/latency` was untouched.

## Method

`growth_probe` is an isolated diagnostic copy of PR22's allocator example. Its explicit `History` contains record count, unique key count, stream count, records per submit, and payload length. Eight-byte stream identifiers cycle round-robin; unique eight-byte keys occur on the first K records; 32-byte payloads encode the record index modulo 251. Every acknowledged offset and revision is checked. Every reopen is followed by full validation of offsets, revisions, streams, keys, and payload bytes. Any error or mismatch fails the process. [final-run.txt](final-run.txt) records 90 successful executions and zero failures.

The runner executes ten cases, three repetitions each, in seed, fresh-process guest-warm, fresh-process guest-dropped order. It retains the same dataset across those three modes, then removes its own data directory. No samples were discarded. The earlier eight-case pilot also passed 72 executions; its raw files remain local and are excluded from the final matrix. These are growth diagnostics with three repetitions, not an implementation comparison or performance improvement claim. Timing variance prevents ranking small differences.

All diagnostic appends use `Options { linger: Duration::ZERO, ..Options::default() }`. Each submit is awaited before the next, giving deterministic group counts. The runner's group counts are derived from validated sequential commits, not an independent frame census. Group count is `ceil(records / group_records)`. All options other than linger retain defaults. Every run finishes far below roll size. This is not the default API benchmark.

Each phase records calling-thread requested allocation traffic, process live requested heap delta, process extra peak requested heap, process live requested heap, wall time in nanoseconds, Linux VmRSS and lifetime VmHWM in KiB. Time includes the operation's allocations, real storage reads or writes, checks inside that operation, locking, and startup or cleanup where applicable. It excludes counter reporting, `/proc` parsing, and subsequent validation. Allocator atomics perturb timings. Writer allocations are excluded from the calling-thread traffic counter and included in process counters. Async writer startup can affect per-phase deltas. Requested bytes exclude allocator size classes, stacks, libc allocations, and the kernel cache. RSS is a phase-end snapshot, not a phase peak. VmHWM is a process-lifetime high-water mark and cannot identify the phase that caused an earlier peak.

`caller_build` retains every input submit before admission. `append_and_release_caller` consumes and drops them while waiting for commits. `drop_before_reopen` releases the first engine. `reopen` opens, scans and validates storage, reconstructs indexes, syncs the recovered tail, and starts the writer. `read_start` and `read_end` each request and validate one record. They run after recovery, so even guest-dropped mode's reads can use data cached during recovery. `validation_buffer` returns the entire history; `validation_check` checks it; `validation_release` drops it; `drop_engine` joins the writer and releases engine state. Its negative requested heap delta is a useful engine-retention proxy, with fixed process effects, not exact ownership attribution.

Guest-warm reopen starts a fresh process without cache eviction immediately after the validated seed process exits. Guest-dropped reopen runs `sync`, then successfully writes `3` to `/proc/sys/vm/drop_caches` as root before starting another fresh process. This globally evicts eligible guest page, dentry, and inode caches. It does not flush hypervisor or device caches, prove physical cold media, model a power loss, or qualify the virtual device as physical NVMe. Root authorized this only during the exclusive experiment interval.

## Results

[phases.csv](phases.csv) contains every final observation. Cells below are median [minimum, maximum] over three repetitions. Heap deltas and allocation traffic are bytes; RSS is KiB; time units appear in headers.

| Case | Records | Keys | Streams | Groups | Warm reopen ms | Guest-dropped reopen ms | Warm reopen RSS KiB | Engine Drop heap bytes |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| records_1000 | 1000 | 0 | 1 | 10 | 2.67 [1.82, 2.70] | 3.39 [2.87, 3.43] | 2612.00 [2612.00, 2616.00] | -264997.00 [-264997.00, -264997.00] |
| records_10000 | 10000 | 0 | 1 | 100 | 10.36 [4.43, 10.50] | 11.69 [11.57, 11.70] | 2664.00 [2620.00, 2672.00] | -269477.00 [-269477.00, -269477.00] |
| records_100000 | 100000 | 0 | 1 | 1000 | 33.85 [33.63, 62.12] | 36.82 [35.74, 36.86] | 2656.00 [2656.00, 2660.00] | -305317.00 [-305317.00, -305317.00] |
| records_1000_fixed_groups | 1000 | 0 | 1 | 100 | 3.40 [3.37, 3.65] | 3.94 [2.86, 3.96] | 2600.00 [2592.00, 2656.00] | -269477.00 [-269477.00, -269477.00] |
| records_100000_fixed_groups | 100000 | 0 | 1 | 100 | 30.56 [30.02, 30.66] | 35.02 [34.65, 35.20] | 2720.00 [2716.00, 2780.00] | -269477.00 [-269477.00, -269477.00] |
| keys_half | 10000 | 5000 | 1 | 100 | 13.81 [6.27, 13.99] | 12.71 [7.28, 14.97] | 3564.00 [3544.00, 3572.00] | -1079733.00 [-1079733.00, -1079733.00] |
| keys_all | 10000 | 10000 | 1 | 100 | 8.46 [7.99, 18.97] | 20.02 [9.28, 20.41] | 4560.00 [4548.00, 4588.00] | -1966773.00 [-1966773.00, -1966773.00] |
| streams_1000 | 10000 | 0 | 1000 | 100 | 5.14 [5.11, 5.34] | 12.66 [12.39, 13.13] | 2720.00 [2720.00, 2720.00] | -353645.00 [-353645.00, -353645.00] |
| groups_1000 | 10000 | 0 | 1 | 1000 | 13.54 [7.03, 16.13] | 8.48 [8.29, 8.49] | 2652.00 [2652.00, 2704.00] | -305317.00 [-305317.00, -305317.00] |
| groups_10000 | 10000 | 0 | 1 | 10000 | 35.31 [35.29, 35.66] | 37.06 [36.81, 62.82] | 3124.00 [3084.00, 3144.00] | -919717.00 [-919717.00, -919717.00] |

At a fixed 100 groups, zero keys, and one stream, 1k, 10k, and 100k records release the same requested heap on engine drop. Reopen still scans the larger history and requests temporary allocations. At 10k records and 100 groups, adding keys or streams increases retained heap independently. At 10k records, zero keys, and one stream, increasing groups increases retained heap. These observations support the existing frame/key/stream ownership explanation. They do not establish an unbounded-memory leak.

The following table uses `guest_warm` observations for reads and validation-buffer deltas. Caller-build deltas use `seed`, the mode that constructs caller submissions.

| Case | Start read us | End read us | Start thread allocation bytes | Caller live delta bytes | Full validation buffer live delta bytes |
| --- | --- | --- | --- | --- | --- |
| records_1000 | 82.51 [34.32, 83.80] | 69.36 [28.51, 74.12] | 17336.00 [17336.00, 17336.00] | 128402.00 [128402.00, 128402.00] | 130112.00 [130112.00, 130274.00] |
| records_10000 | 97.98 [34.43, 143.65] | 70.95 [28.53, 80.75] | 17336.00 [17336.00, 17336.00] | 1282562.00 [1282544.00, 1282562.00] | 1841792.00 [1841792.00, 1841930.00] |
| records_100000 | 38.71 [36.90, 40.84] | 29.45 [29.05, 29.48] | 17336.00 [17336.00, 17336.00] | 12824138.00 [12824138.00, 12824162.00] | 15534498.00 [15534336.00, 15534498.00] |
| records_1000_fixed_groups | 26.11 [25.67, 27.75] | 12.74 [12.71, 13.34] | 2216.00 [2216.00, 2216.00] | 130400.00 [130394.00, 130562.00] | 130112.00 [130112.00, 130112.00] |
| records_100000_fixed_groups | 268.98 [267.86, 287.72] | 258.46 [258.11, 306.73] | 168536.00 [168536.00, 168536.00] | 12802562.00 [12802400.00, 12802562.00] | 15534336.00 [15534336.00, 15534336.00] |
| keys_half | 84.90 [47.90, 108.19] | 57.02 [29.61, 71.66] | 18944.00 [18944.00, 18944.00] | 1322538.00 [1322400.00, 1322562.00] | 1881792.00 [1881792.00, 1881792.00] |
| keys_all | 45.27 [43.38, 99.91] | 32.93 [32.01, 78.98] | 18944.00 [18944.00, 18944.00] | 1362562.00 [1362562.00, 1362562.00] | 1921936.00 [1921792.00, 1921936.00] |
| streams_1000 | 38.32 [37.61, 65.57] | 28.58 [28.25, 28.86] | 17336.00 [17336.00, 17336.00] | 1282562.00 [1282562.00, 1282562.00] | 1841936.00 [1841936.00, 1841954.00] |
| groups_1000 | 21.39 [12.95, 26.68] | 10.55 [5.76, 13.42] | 2216.00 [2216.00, 2216.00] | 1304162.00 [1304000.00, 1304162.00] | 1841792.00 [1841792.00, 1841792.00] |
| groups_10000 | 13.89 [13.44, 14.24] | 3.96 [3.69, 4.32] | 704.00 [704.00, 704.00] | 1520138.00 [1520000.00, 1520144.00] | 1841954.00 [1841954.00, 1841954.00] |

A one-record read still reads and decodes its whole selected group. The fixed-group record-growth cases increase group size and therefore read allocation traffic. Start and end reads do not clone unrelated frame history after PR22. Timings above are individual reads, not read latency distributions under sustained load. No throughput, p99, limiter attribution, or before/after speed claim follows from these samples.

RSS and requested heap tell different stories. Freed caller and validation buffers can leave reusable allocator pages resident. The large full-history validation can dominate VmHWM even after engine drop. Fresh-process reopen snapshots avoid the seed process's caller-allocation history; they still include the runtime, allocator, thread stacks, recovery scratch retention, and resident engine pages. RSS is neither engine heap size nor an admission-budget ceiling. No malloc trim or allocator replacement was applied.

## Default API and checks

[default-api.txt](default-api.txt) preserves one separate release invocation of the existing `measured_appends_survive_reopen` benchmark on this PR22-based library. It uses default 1 ms linger, four callers, four rounds, 16 acknowledged records, four groups, and validates the reopened results. It is a small default API baseline with one invocation, not a comparison with diagnostic linger zero. Its device micro-measurement is virtual-guest evidence only.

[checks.txt](checks.txt) preserves successful Linux commands at the identical Rust source:

```sh
cargo test --release --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --release -p segment-log-core measured_appends_survive_reopen -- --nocapture
```

The workspace run passed 135 unit tests, three process tests, the read-range integration test, and the real-storage integration test. One scheduled long-run unit test stayed ignored. The read-range test includes 64 concurrent read/append rounds. No new concurrent read/append latency or RSS measurement was attempted in this bounded matrix. Independent review remains pending; no subagents were authorized. No observed defect required a production fix.

## Reproduce

Use the pinned toolchain on an otherwise idle, disposable Linux host. Cache eviction is global and requires an exclusive interval plus root. Omit `--drop-guest-caches` to run only seed and guest-warm cases when that is inappropriate. Output and data paths must be new.

```sh
git switch --detach 531843ea02c1b8d6dd54cea2bb980e250e946633
cargo build --release -p segment-log-core --example growth_probe
python3 scripts/m1-growth-probe.py \
  --binary target/release/examples/growth_probe \
  --output .audit/growth --repetitions 3 --drop-guest-caches
cargo test --release -p segment-log-core measured_appends_survive_reopen -- --nocapture
```

The captured final command ran from `/root/segment-log-memory` with `--output evidence-final`; other arguments were identical. The runner saves individual stdout, all phase rows, host metadata, and subprocess commands. The successful stdout marker counts runs and validated group/record totals. No local Mac timing was measured or claimed. Source transfer used `COPYFILE_DISABLE=1 tar` excluding `.git`, `target`, `.audit`, and `._*`. Linux tar reported ignored provenance xattr headers; no AppleDouble files entered the build. Remote evidence was copied to local ignored `.audit/memory-growth` before releasing the interval. [artifacts.sha256](artifacts.sha256) identifies the captured archives and raw phase file.

Remaining gaps are sustained concurrent read/append performance, histories beyond 100k records, different payload/key sizes, multi-segment retention, workload distributions, recovery after actual power loss, long-run soak, and physical storage qualification. Three diagnostic repetitions cannot establish a performance improvement. These results do not replace the five comparable repetitions required for such a claim.
