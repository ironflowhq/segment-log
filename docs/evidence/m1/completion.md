# M1 completion record

This record supplies the evidence that the [M1 completion gate](../../architecture/testing.md#m1-completion-gate) requires, for the tree that this pull request leaves on `main`. It replaces [slice 10](slice10.md) as the completion record. Slice 10 measured the tree before the issue 20 changes, and its benchmark had no paced run.

The owner decides the gate in [issue 17](https://github.com/ironflowhq/segment-log/issues/17). This record does not mark M1 complete.

## Result

| Gate item | Evidence | Result |
| --- | --- | --- |
| 1. Checks on Linux and macOS | Hosted and local captures, long runs, hosted CI | Pass on both platforms |
| 2. One `fdatasync` for each new group | Linux syscall trace on two hosts, with the verifier | Pass |
| 3. Directory syncs during creation and recovery | Linux trace, macOS storage results, hosted and local | Pass on both platforms |
| 4. Reproducible Linux benchmark report | Default and paced runs on two Linux hosts | Present, with deviations recorded |

No correctness or durability failure was observed. One long run failed during this work. Its cause was in the test harness and is corrected ([record](../issue20/reader-crash-cancel/README.md)).

The performance targets are diagnostics in M1 ([ADR 0029](../../decisions/0029-m1-completion-requires-correctness-and-measurement-evidence.md)). Neither Linux host is local NVMe, so no target is claimed as passed.

- **Throughput target, at least 50,000 records/s.** The closed loop measured a median of 198,198 records/s on the hosted runner and 424,813 on the Docker guest.
- **Latency target, p99 within the linger plus twice the device `fsync` p99.** At the paced 50,000 records/s the p99 was within the bound of the same run in 4 of 5 runs on the Docker guest, and in 0 of 5 on the hosted runner. The misses follow slow device flushes, which the tables below count.

## Source

| | |
| --- | --- |
| Measured commit | `fda412be697d7f65941cd27074d78cf51c9e6963` |
| `segment-log-core` tree | `e4dc23bcc919d1a5360634ceb1454b64c75f188d` |
| `scripts` tree | `82799e7f2569367fb73e91c55b990b91231492fe` |
| `.github` tree | `bdbd2732b26869c0fca235a22627ba3eb98e09c8` |
| Toolchain | Rust 1.99.1 (`58cf8f9c3`, LLVM 23.1.1), from `rust-toolchain.toml` |

A later commit in this pull request must leave the `segment-log-core` tree unchanged for this record to describe it. Compare with `git rev-parse HEAD:segment-log-core`.

All four captures ran on 10 November 2026:

| Capture | Host | Files |
| --- | --- | --- |
| Hosted Linux | GitHub runner `ubuntu-24.04`, [run 42763429080](https://github.com/ironflowhq/segment-log/actions/runs/42763429080) | [completion/hosted-linux/](completion/hosted-linux/) |
| Hosted macOS | GitHub runner `macos-latest`, the same run | [completion/hosted-macos/](completion/hosted-macos/) |
| Docker guest | Pinned `rust:1.99.1-bookworm` container on Docker Desktop for Mac | [completion/docker/](completion/docker/) |
| Mac host | Apple M4, macOS 27.0.1 | [completion/macos/](completion/macos/) |

[completion/hosted-run.json](completion/hosted-run.json) is the API record of the hosted run. Three local files were edited after capture to remove directory names of the host: line 2 of `docker/environment.txt`, and the process paths in `docker/host-load-before.txt` and `docker/host-load-after.txt`, which now hold process names only.

## 1. Checks on Linux and macOS

Every capture ran `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the script tests, the case-coverage check and a `--cfg fuzzing` check. Every capture ended with `exit=0`.

| Capture | Library tests | Process tests | Read-range tests | Real-storage tests | Script tests |
| --- | --- | --- | --- | --- | --- |
| Hosted Linux | 152 passed, 1 ignored | 3 | 2 | 1 | 20 |
| Hosted macOS | 152 passed, 1 ignored | 3 | 2 | 1 | 20 |
| Docker guest | 152 passed, 1 ignored | 3 | 2 | 1 | 20 |
| Mac host | 152 passed, 1 ignored | 3 | 2 | 1 | 20 |

The ignored test is the long run. It ran separately with the hosted settings, 100 trace seeds from 10000 and 30 process-kill repetitions:

| Host | Trace oracle | Process kills |
| --- | --- | --- |
| Mac host | 2,700 scenarios passed | 30 repetitions passed |
| Docker guest | 2,700 scenarios passed | 30 repetitions passed |

[completion/long-run/](completion/long-run/) holds the four logs. The tests cover the simulated `Kill`, `PowerLoss` and `PowerLossTorn` crashes, real-process `SIGKILL`, I/O faults, recovery, the writer lock, expected revisions, retries and admission limits. [testing.md](../../architecture/testing.md#m1-acceptance-coverage) lists every case, and each group there is `Implemented`.

Hosted CI passed on Linux and macOS for the head of each pull request in this stack: #36, #37, #38, #40, #41 and #42.

### Hosted long runs

| Run | Trigger | Result |
| --- | --- | --- |
| [42654692594](https://github.com/ironflowhq/segment-log/actions/runs/42654692594) | Manual, 9 November | Passed on Linux and macOS |
| [42754928006](https://github.com/ironflowhq/segment-log/actions/runs/42754928006) | Manual, 10 November, after the workflow change | Passed on Linux and macOS |
| [42754924923](https://github.com/ironflowhq/segment-log/actions/runs/42754924923) | Manual drill, 10 November | Both platforms failed as planted, uploaded, and replayed ([record](../issue20/hosted-drill/README.md)) |

No scheduled run exists yet. The first was due on 10 November 2026 at 04:23 UTC. More than one hour later the repository showed no run with the `schedule` event. GitHub does not promise when a scheduled workflow starts. This gate does not require a scheduled run, and the point stays open in issue 20.

## 2. One `fdatasync` for each new group

`strace -f -tt -T -yy` traced the scenario `real_storage_sync_phases`, and `scripts/m1-verify-syscalls.py` checked the calls and the files that they named. The counts are the same on both Linux hosts:

| Phase | Segment `fdatasync` | Segment and directory `fsync` |
| --- | --- | --- |
| Creation | 1 | 4 |
| First new group | 1 | 0 |
| Second new group | 1 | 0 |
| Fully deduplicated retry | 0 | 0 |
| Drop | 0 | 0 |
| Recovery on reopen | 0 | 3 |
| Drop after recovery | 0 | 0 |

Creation and recovery are separate from the group commits. A fully deduplicated retry adds no group and no flush. The provisioning phase before creation belongs to the test, which makes its temporary ancestors durable.

| Host | Trace | Verifier output | Phase log |
| --- | --- | --- | --- |
| Hosted Linux, x86-64 | [syscalls.trace](completion/hosted-linux/syscalls.trace) | [syscalls-verified.txt](completion/hosted-linux/syscalls-verified.txt) | [sync-phases.log](completion/hosted-linux/sync-phases.log) |
| Docker guest, aarch64 | [syscalls.trace](completion/docker/syscalls.trace) | [syscalls-verified.txt](completion/docker/syscalls-verified.txt) | [sync-phases.log](completion/docker/sync-phases.log) |

Command, from `scripts/m1-evidence-guest.sh`:

```sh
strace -f -tt -T -yy -s 256 -e trace=fdatasync,fsync,write -o syscalls.trace \
    TEST_BINARY --exact acceptance::evidence::real_storage_sync_phases --nocapture
python3 scripts/m1-verify-syscalls.py syscalls.trace
```

In the benchmark, the storage wrapper counts one successful group flush for each group that it sees appended, in every run. A run fails if the two counts differ.

## 3. Directory syncs on both platforms

The same scenario checks the directory syncs through the real storage implementation: two for the log directory and one for its parent during creation, and one each during recovery.

- **Linux.** The traces above show each `fsync` on the log directory and on its parent return 0.
- **macOS.** [completion/hosted-macos/directory.log](completion/hosted-macos/directory.log) and [completion/macos/directory.log](completion/macos/directory.log) record `Ok(())` for every `SyncDir` call on the log directory and on its parent, during creation and recovery. These are results of the real storage calls on APFS, not a kernel trace.

These checks show the calls that the library makes. They do not certify a storage stack. [ADR 0032](../../decisions/0032-qualify-deployment-storage-before-m6.md) keeps that qualification before M6.

## 4. Linux benchmark report

### Conditions

| | Hosted Linux runner | Docker guest |
| --- | --- | --- |
| Processor | AMD EPYC 7763, 4 processors, x86-64 | 10 virtual processors of an Apple M4, aarch64 |
| Memory | 16,760,344,576 bytes | 8,319,438,848 bytes |
| Kernel | Linux 7.0.0-1017-azure, Ubuntu 24.04.5 | Linux 7.0.14-linuxkit, Debian 12 container |
| Hypervisor | Microsoft, an Azure virtual machine | Docker Desktop on macOS 27.0.1, Engine 29.9.3 |
| Device | `nvme0n1`, 150 GB, model "MSFT NVMe Accelerator v1.0" | Virtual disk `/dev/vda1` in a file on the Mac |
| Filesystem of the data | ext4 on `/dev/root` | The container's overlay on ext4 |
| Mount options | `rw,relatime,discard,journal_async_commit,nobarrier,errors=remount-ro,commit=30,data=writeback` | `rw,relatime,discard,noauto_da_alloc` for the ext4 below the overlay |
| Volatile write cache | Not established. `nobarrier` means ext4 sends no cache flush to the device, so `fdatasync` does not force one there | Not established. Flush propagation to the Mac's media is unknown |
| Other load | A shared virtual machine host. Not visible | The guest was idle before every run ([load files](completion/docker/)). The Mac was quiet at the start and at the end of the capture |

Deviations from the target conditions, on both hosts: no local NVMe, a virtual machine, and no qualified flush. The hosted runner has 4 processors for 1,000 caller threads. The hosted mount does not force a device flush, so its flush times describe that mount, not a durable device.

All runs use `Options::default()`: linger 1 ms, group cap 1,048,576 bytes, queue capacity 4,096, admission budget 16,777,216 bytes, and the inactive roll size 67,108,864 bytes. Each run has 1,000 caller threads, 1,000 rounds, one keyed 256-byte record for each submit, and so 1,000,000 records. The workload has no random input. [Slice 10](slice10.md#benchmark-design-and-correctness) describes its design.

Each run first measures the device alone: 1,000 appends of 262,144 bytes to a growing file, with `fdatasync` timed after each.

### Default runs: the throughput target

The default workload is a closed loop. [hosted-linux/summary.md](completion/hosted-linux/summary.md) and [docker/summary.md](completion/docker/summary.md) hold every run.

| | Hosted Linux runner, median (range) | Docker guest, median (range) |
| --- | --- | --- |
| Records/s | 198,198 (196,038–206,657) | 424,813 (392,853–430,992) |
| Append p50 ms | 4.906 (4.647–4.920) | 2.301 (2.280–2.404) |
| Append p99 ms | 8.102 (7.617–8.861) | 3.425 (3.346–3.549) |
| Append p999 ms | 16.874 (11.678–18.145) | 4.903 (4.042–80.296) |
| Device `fdatasync` p50 ms | 0.312 (0.298–0.458) | 0.308 (0.298–0.310) |
| Device `fdatasync` p99 ms | 0.952 (0.714–1.913) | 0.457 (0.358–0.603) |
| Reopen ms | 859.8 (847.6–887.0) | 475.1 (469.7–491.4) |

- **Target comparison.** Both medians are above 50,000 records/s. This is not a pass under the target conditions.
- **Closed-loop latency.** The mean equals the caller count divided by the throughput, to within 0.05 ms in every run. It is a diagnostic and not the target comparison ([ADR 0045](../../decisions/0045-latency-target-is-measured-at-a-paced-rate.md)).

### Paced runs: the latency target

Each caller submits once in 20 ms, so the callers offer 50,000 records/s, and latency runs from the scheduled send.

| Host and run | p50 ms | p99 ms | p999 ms | Max ms | Bound ms | p99 minus bound ms | Group flushes of 2 ms or more |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Hosted 1 | 1.192 | 2.758 | 13.456 | 21.371 | 2.465 | +0.293 | 38 |
| Hosted 2 | 1.218 | 4.221 | 15.799 | 22.137 | 2.344 | +1.876 | 43 |
| Hosted 3 | 1.213 | 3.834 | 13.153 | 17.679 | 2.362 | +1.472 | 49 |
| Hosted 4 | 1.225 | 3.449 | 12.642 | 17.371 | 2.067 | +1.382 | 41 |
| Hosted 5 | 1.175 | 4.044 | 14.533 | 21.072 | 2.671 | +1.373 | 50 |
| Docker 1 | 1.112 | 46.079 | 85.042 | 98.478 | 1.994 | +44.084 | 336 |
| Docker 2 | 1.046 | 1.669 | 2.143 | 8.087 | 1.688 | −0.018 | 1 |
| Docker 3 | 1.005 | 1.797 | 27.467 | 45.046 | 1.994 | −0.198 | 14 |
| Docker 4 | 1.035 | 1.642 | 2.260 | 5.276 | 1.763 | −0.122 | 4 |
| Docker 5 | 0.966 | 1.520 | 1.695 | 4.359 | 1.779 | −0.258 | 2 |

The bound is the linger plus twice the device `fdatasync` p99 that the same run measured. Every run kept up with the offered rate.

- **Docker guest.** The p99 is within the bound in runs 2 to 5, by 0.018 to 0.258 ms. Run 1 is a device stall: 336 of its 16,866 group flushes took 2 ms or more, and the longest took 59.8 ms.
- **Hosted runner.** The p99 misses the bound in all five runs, by 0.293 to 1.876 ms. In each run 38 to 50 of about 18,700 group flushes took 2 ms or more, up to 15.2 ms, although the flush p99 was 0.66 to 0.91 ms. The submits in such a flush or in the group right after it are 0.93 to 1.46% of the million. That is enough to set the p99.
- **What this shows.** The bound uses the flush p99, so flushes slower than that are outside it. The runs with 1 to 14 such flushes are within the bound. The runs with 38 or more are not. [ADR 0045](../../decisions/0045-latency-target-is-measured-at-a-paced-rate.md) names this limit.
- **Target comparison.** Four runs of five within the bound on one virtual guest is not a pass under the target conditions.

The summaries also give the same comparison with the p99 of each run's own group flushes in place of the device measurement. A paced group is about 16 KiB, and the device measurement syncs 256 KiB appends. The result is the same: 4 of 5 on the Docker guest and 0 of 5 on the hosted runner.

### Other required fields

| Field | Hosted Linux runner | Docker guest |
| --- | --- | --- |
| Admission refusals | 0 in all ten runs | 0 in all ten runs |
| Peak outstanding charges, default runs | 544,000 bytes in each | 544,000 bytes in each |
| Peak outstanding charges, paced runs | 426,496–525,504 bytes | 114,784–544,000 bytes |
| Groups, default runs | 2,009–2,018 | 1,316–1,414 |
| Submits for each group, default runs, p50 and max | 499–501 and 954–997 | 885–1,000 and 1,000 |
| Frame bytes for each group, default runs, p50 and max | 147,736–148,328 and 282,416–295,144 | 261,992–296,032 and 296,032 |
| Groups, paced runs | 18,641–18,761 | 16,866–19,017 |
| Submits for each group, paced runs, p50 and max | 53 and 632–765 | 52–53 and 161–952 |
| Records and segments at the start | 0 records, 1 segment, 64 bytes | The same |
| Records and segments at the end | 1,000,000 records, 1 segment, 296,064,352–296,600,416 bytes | 1,000,000 records, 1 segment, 296,042,176–296,608,608 bytes |
| Reopen | The same records and bytes before and after, 834–887 ms | The same, 447–491 ms |
| First open of the empty log | 0.86–1.45 ms | 0.85–0.95 ms |
| Whole-process maximum resident set | 609,960–662,144 KiB | 610,812–660,652 KiB |

The peak charge is 544 bytes for each of 1,000 callers. It was 480 bytes in slice 10. The request's reply state ([ADR 0044](../../decisions/0044-woken-callers-relay-group-acks.md)) and its admission time ([ADR 0046](../../decisions/0046-linger-runs-from-the-first-submits-admission.md)) account for the difference. The `groups-N.tsv` and `paced-groups-N.tsv` files list every group with its submits, its bytes and its flush time. The resident set is for the whole test process with its 1,000 caller stacks and its verification buffers. It is not an idle resident set.

Every run validated all its records against the literal inputs and the callers' positions, before and after the reopen, with no commit error and no correctness error.

### Not applicable in M1

- Open time with 100 sealed segments and footers, and seal-to-exit time: M4.
- Idle resident set and binary size of the server, and socket measurements: M6.
- Roll behavior and its syscall trace: M3. An M1 log has one segment that grows for the whole run.

### A second hosted capture with a stalling device

[completion/hosted-linux-stalled/](completion/hosted-linux-stalled/) is an earlier capture, [run 42762336987](https://github.com/ironflowhq/segment-log/actions/runs/42762336987) at `edefd1c`, on a runner with an AMD EPYC 9V74. The engine code is the same as in the measured commit. Its device `fdatasync` p99 was 4 to 110 ms and its paced p99 was 219 to 656 ms. It is kept because a report gives every paced run. It shows how far one hosted runner's disk can differ from the next.

## Findings and limits carried forward

- **No physical local NVMe.** The targets name it, and no host here has it. [The pull request](https://github.com/ironflowhq/segment-log/pull/40) lists ways to get that measurement.
- **Deployment storage is not qualified.** That is a requirement before M6 ([ADR 0032](../../decisions/0032-qualify-deployment-storage-before-m6.md)), not part of this gate.
- **The paced p99 depends on rare slow flushes.** See the tables above.
- **ADR 0046 is proposed.** The measured tree counts a group's linger from its first submit's admission. If the owner rejects that decision, capture this record again without it.
- **Exact peak memory for each phase is a lower bound on this guest.** See the [Linux phase record](https://github.com/ironflowhq/segment-log/pull/39).
- **Workloads beyond these stay unmeasured:** more than 1,000 callers, more than 1,000,000 records, and long soaks, which need roll (M3).
- **The scheduled long run has not started yet.**
- **Missing original red evidence stays missing.** Some early behavior was implemented before its scenario ([evidence archive](README.md)). Later tests do not repair that history.

## Reproduce

Hosted, with no local Linux host: put the `m1-evidence` label on a pull request, or dispatch the `M1 evidence capture` workflow once it is on `main`. Download the artifacts with `gh run download RUN_ID`.

Local, from the repository root, with Docker running:

```sh
scripts/m1-evidence-linux.sh "$PWD/.audit/completion/docker"
scripts/m1-evidence-macos.sh "$PWD/.audit/completion/macos"
python3 scripts/m1-summarize.py .audit/completion/docker
python3 scripts/m1-verify-syscalls.py .audit/completion/docker/syscalls.trace
SEGMENT_LOG_TRACE_SEEDS=100 SEGMENT_LOG_TRACE_SEED=10000 \
  cargo test -p segment-log-core --lib acceptance::seeded::threaded_seeded_traces_long_run -- --ignored --exact --nocapture
SEGMENT_LOG_PROCESS_REPETITIONS=30 SEGMENT_LOG_PROCESS_SEED=10000 \
  cargo test -p segment-log-core --test process_crash killed_writers_preserve_the_recorded_contract -- --nocapture
```

On another Linux host, run `scripts/m1-evidence-guest.sh` from the repository root with `M1_OUT`, `CARGO_TARGET_DIR` and `SEGMENT_LOG_EVIDENCE_DIR` set. The last one chooses the filesystem under test.
