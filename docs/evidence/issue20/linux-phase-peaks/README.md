# Linux phase peaks: resident memory at each phase for issue 20

[Issue 20](https://github.com/ironflowhq/segment-log/issues/20), section 2, asks for steady-state and peak memory, measured separately for the engine, the caller workload, validation, read and reopen. The [Mac record](../mac-resident/README.md) separates those phases, but it samples the resident set with `ps`, so most short operations have no peak. This record reads the Linux kernel's own counters at each phase barrier instead. It adds a driver and its tests. It changes no production code, default, API, format or dependency.

## Result

**Steady state: read at every barrier, without sampling.** The matrix is 60 fresh processes with 312 barriers: five probe modes on four histories, three repetitions each. In 91 of the 104 cells all three repetitions give the same resident set above the process's own baseline. In 102 cells they are within 12 KiB of the median. The other two cells are validation reads with one repetition 660 and 1,028 KiB higher.

Whole-process resident set above the same process's baseline, in KiB. A cell is the median [range] of three processes. One number alone means that all three gave it.

| Phase end | Mode, barrier | 16,384 records | 16,384 records, all keyed | 1,000,000 records | 1,000,000 records, all keyed |
| --- | --- | ---: | ---: | ---: | ---: |
| Control: one buffer held | touch, `touched_held` | 520 | 516 | 31,256 | 31,252 |
| Caller workload: every submit held, no engine | caller, `caller_held` | 2,692 | 3,204 | 164,104 | 195,352 |
| Caller workload dropped | caller, `caller_dropped` | 2,692 | 3,204 | 164,104 | 195,352 |
| Engine: empty log open | empty, `empty_held` | 316 [304–316] | 316 | 316 [308–316] | 316 [304–316] |
| Engine: every group appended | append, `appended_held` | 400 | 2,132 | 908 | 101,432 [101,428–101,432] |
| Read: one record held, after the append | append, `read_one_held` | 412 | 2,132 | 908 | 101,484 |
| Reopen: history recovered | recover, `recovered_held` | 356 [352–360] | 2,076 | 636 [624–636] | 101,112 [101,100–101,112] |
| Read: one record held, after the reopen | recover, `read_one_held` | 360 | 2,076 | 636 | 101,112 |
| Validation: every record held, after the reopen | recover, `validation_held` | 3,040 | 5,268 | 164,764 [164,764–165,792] | 296,464 [296,464–296,468] |
| Validation result dropped | recover, `validation_dropped` | 1,628 | 3,856 | 78,736 | 210,440 |
| Engine dropped | recover, `engine_dropped` | 1,592 | 3,820 | 78,700 | 210,404 |

The baseline itself is 1,812 KiB (1,796–1,816 in 60 processes), of which 1,696 KiB are file-backed pages of the executable and its libraries.

- **Engine.** A process with an empty log open holds 316 KiB more than its baseline (304–316 in 24 processes).
- **Reopen.** A process that reopened 1,000,000 records holds 636 KiB above its baseline without keys and 101,112 KiB with 1,000,000 keys. With 16,384 records it holds 356 and 2,076 KiB.
- **Keys, as a difference between two processes.** Keyed minus unkeyed at the reopen barrier is 1,720 KiB for 16,384 keys and 100,476 KiB for 1,000,000 keys. That is 107.5 and 102.9 resident bytes for each key. The [key positions record](../key-positions/README.md) counted 81.9 and 77.7 requested heap bytes for each key on an earlier commit.
- **Caller workload and validation.** They are far larger than the engine without keys. Holding every submit of 1,000,000 records takes 164,104 KiB with no engine in the process. The validation read adds 164,128–165,156 KiB to the reopened process.
- **Read.** A one-record read adds 0–56 KiB in 24 processes.

**Peak: the kernel recorded no peak above the ends of any phase.** A phase runs from one barrier to the next. In all 252 phases of the matrix, the kernel's mark (`VmHWM`) is never above both the resident set at the start of the phase and the resident set at its end. So the open, append, read, reopen and validation-read phases peak at their own steady state, and each drop phase peaks at the resident set that it starts with. [summary.md](summary.md) gives the steady state and the peak of every phase.

**The kernel's mark is not exact here, so a small transient can hide.** The resident set fell in 73 phases, the reset check included. The peak of such a phase is at least the resident set at its start, which the driver had just read. The mark was below that value in 67 of the 73, by 36–576 KiB. A transient peak above the ends of a phase is therefore not excluded. Where the mark could be checked, it missed at most 576 KiB. The steady-state readings do not have this error.

This gives each phase a steady state, and a peak with a lower bound, on one Linux guest. It does not give exact transient peaks, engine-owned bytes, or any figure for target hardware. It does not decide a PRD target.

## Scope and provenance

- The Rust sources are `origin/main` at `86f8da8f9450db26731c9e72d0dc13c22fe3802a`. The work tree held only this record's new files beside it ([sources.txt](sources.txt)). `resident_probe` is unchanged. [binaries.sha256](binaries.sha256) gives the hash of the probe binary and of the driver that ran.
- Captured on 10 November 2026 at 04:25 UTC in the pinned `rust:1.99.1-bookworm` image (`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) on Docker Desktop for Mac. The guest runs Linux 7.0.14-linuxkit on aarch64 with 10 processors, 8,319,438,848 bytes of memory, 4 KiB pages, glibc 2.36, transparent huge pages set to `always`, and 1 GiB of swap with none used. [environment.txt](environment.txt) and [docker-info.txt](docker-info.txt) hold the details.
- The data directory was `/data/peaks` on the container's overlay filesystem, on the guest's virtual disk. The build was on the same filesystem. The sources were a read-only bind mount and the output directory was a bind mount. The probe wrote to neither.
- The host is an Apple M4 with 10 cores and 24 GiB on macOS 27.0.1 ([host.txt](host.txt)). It ran unrelated work, and other sessions' containers ran in the same guest. [docker-stats-before.txt](docker-stats-before.txt) and [docker-stats-after.txt](docker-stats-after.txt) show them around the matrix. The run was not gated on load, because a resident-set reading at a barrier does not depend on timing.
- [run.sh](run.sh) holds the exact commands. It ran once and took 33 seconds. Three things ran before it and are not in this record: a check of the reset by hand with a 64 MiB Python buffer, two pilots of one repetition on one history each, and one attempt of `run.sh` that stopped in its environment step before any probe started.
- After the run, `raw.tar.gz` was packed again from the same files with `root` as the file owner, and the last line of `run.sh` was changed to pack inside the container. Nothing else in `run.sh` changed after it ran.
- This is a virtual Linux guest on a Mac. It is not target hardware.

## Method

`resident_probe` prints a barrier line at the end of each phase and waits on stdin. `scripts/m1-linux-phase-peaks.py` does four things at each barrier, then sends `go`:

1. It reads `/proc/<pid>/status` and keeps `VmRSS`, `VmHWM`, `RssAnon`, `RssFile`, `RssShmem` and `VmSwap`.
2. It reads `/proc/<pid>/smaps_rollup`, which the kernel computes by walking the page tables.
3. It writes `5` to `/proc/<pid>/clear_refs`. That resets the high-water mark and nothing else.
4. It reads `status` again, to see what the reset left.

`VmRSS` at a barrier is the steady state of the phase that ends there. `VmHWM` at a barrier is the kernel's mark since the reset at the barrier before it. The reported peak of a phase is the larger of that mark and the resident set at the start of the phase. Every raw text is kept. All sizes are in KiB, as the kernel prints them.

The phases of issue 20 map to the probe like this:

| Issue 20 phase | Probe mode | Phase, and the barrier that ends it |
| --- | --- | --- |
| Engine | `empty`, `append` | Open a new log (`empty_held`). Append every group, one submit at a time, each awaited (`appended_held`). |
| Caller workload | `caller` | Build every submit of the history with no engine in the process (`caller_held`), then drop them (`caller_dropped`). |
| Read | `append`, `recover` | Read one record (`read_one_held`), then drop it (`read_one_dropped`). |
| Validation | `append`, `recover` | Read every record (`validation_held`), then check and drop them (`validation_dropped`). |
| Reopen | `recover` | Reopen the log that the append process of the same repetition wrote, in a new process (`recovered_held`). |
| Control | `touch` | Allocate and fill one buffer of records × payload bytes (`touched_held`), then drop it (`touched_dropped`). |

Every mode starts with a `baseline` barrier before it allocates anything for its work. The `caller` and `touch` modes and that barrier are the controls: they show what the process holds without an engine.

The matrix has four histories. Two are the base and keyed rows of the Mac record's comparison table: 16,384 records in groups of 256 on one stream, with no keys and with 16,384 keys. Two are the larger histories of the key positions record: 1,000,000 records in groups of 1,000 on 1,000 streams, with no keys and with 1,000,000 keys. Payloads have 32 bytes. Keys and stream ids have eight bytes. Linger is zero, and every other option keeps its default.

Each repetition runs the five modes of each history in order, each in a fresh process. The driver refuses a process unless it exits with status 0, passes every barrier of its mode in order, and reports the expected counts: every append process commits and validates every record, and every recover process validates every record. All 61 processes passed.

Before the matrix the driver checks the reset with a known size. It runs the `touch` mode with a 64 MiB buffer. The resident set grew by 65,544 KiB, the mark at the drop was 67,356 KiB, and after the reset both the mark and `VmRSS` were 1,816 KiB.

## Tables

[summary.md](summary.md) is the output of the driver's `summary` command: one table for each history, with every barrier of every mode. Its columns are the steady resident set, the phase peak, the anonymous and file-backed parts, and the steady state and the peak above the same process's baseline, each as a median and full range. [readings.csv](readings.csv) has one row for each barrier of each process, with the kernel's mark beside the reported peak. [raw.tar.gz](raw.tar.gz) holds one JSON file for each process: its command, every line the probe printed, its stderr, and the raw text of every `/proc` read.

More differences between barriers, each from the readings of one process:

- Dropping the engine changed the resident set by −36 KiB in all 36 engine processes: −228 KiB anonymous and +192 KiB file-backed.
- Dropping every submit of the caller workload returned 0 KiB in all 12 caller processes. Dropping the single buffer of the control returned all of it, within 4 KiB.
- Dropping the validation result returned 1,412 KiB with 16,384 records and 86,020–87,056 KiB with 1,000,000 records, keyed or not. After that drop, the process with 1,000,000 unkeyed records still held 78,736 KiB above its baseline, although it held 636 KiB before the validation read. Freed memory does not have to leave the resident set, so a reading after validation says nothing about the engine.
- The anonymous part carries every difference above. The file-backed part is 1,696 KiB at the baseline, 1,760 KiB with an engine and 1,952 KiB after the engine's drop, within 16 KiB.

## The instrument

The last section of [summary.md](summary.md) counts what the 315 barriers of the 61 processes say about the instrument.

- **`VmRSS` agrees with the page tables.** `VmRSS` equals the `Rss` of `smaps_rollup` at 311 barriers. At the other four it differs by 4–8 KiB.
- **The probe is not always still at a barrier.** The driver does not wait before it reads. At five barriers, `VmRSS` grew by 4–12 KiB between the two `status` reads. All five are `empty_held` or `recovered_held`, right after the engine starts its writer thread. This is the resolution of a steady-state reading.
- **The reset works.** No reset left the mark above `VmRSS`.
- **The mark reads low.** The mark was below the resident set at the start of the phase in 67 of the 73 phases where the resident set fell. In the 31 of those phases that fell by more than 1 MiB, the mark was 0–576 KiB low, with a median of 140 KiB. In the 13 control processes, which have one thread, it was 0–124 KiB low. In the 36 engine drops the resident set fell by 36 KiB and the mark missed all of it.
- **No swap and no shared memory.** `VmSwap`, `Swap` and `RssShmem` are zero at every barrier.
- **Transparent huge pages** back 28,672–63,488 KiB at 19 barriers. All 19 hold a control buffer or a validation result of 1,000,000 records. No engine or reopen barrier has any.
- **The control matches its size.** The resident set grew by the buffer's size plus 2–8 KiB in all 13 control processes.

A likely cause of the low mark: upstream Linux counts resident pages in batches for each processor, and it updates the mark from the batched total, while `status` prints `VmRSS` from an exact sum. With upstream's batch of 32 pages, the mark can miss up to 124 KiB for each processor that the process ran on. The 124 KiB limit of the one-thread controls fits that. This record did not inspect the source of this kernel, so the cause and the bound are not established.

## Limits

- **Peaks have a lower bound, not an exact value.** The peak of a phase is at least the larger resident set at its two ends. The kernel recorded nothing above that. Its mark was up to 576 KiB low where that could be checked, and it may be lower where it could not. Pinning the probe to one processor might reduce the error. That was not tried.
- **Whole-process resident set, not engine-owned bytes.** A resident set belongs to a process. Without a dedicated allocator arena for the engine, ownership by component is not defined. Every engine figure here is a difference between two readings of one process or of two processes. It includes the writer thread's stack, allocator overhead and code pages, and it excludes pages that the allocator kept from earlier work.
- **Requested heap is a separate instrument.** `memory_probe` counts requested bytes exactly. The per-key figures quoted from the key positions record are from commit `fcca44aa272f9f65904a76ae1ae069db5caa5bf0` and were not measured again here.
- **One guest, not target hardware.** The numbers depend on glibc 2.36 malloc, 4 KiB pages and transparent huge pages set to `always`. The Mac record used 16 KiB pages, another allocator and an earlier commit, so its figures are not comparable with these.
- **No PRD target is decided.** The PRD's idle resident set target is for the M6 server binary with an empty log. It does not apply to M1, and the 316 KiB of the empty engine here is not a measurement of it.
- **Warm cache only.** Each reopen reads a log that another process wrote a moment before. The page cache is not in a process's resident set, and no cold-cache reopen ran.
- **Three repetitions, four histories.** No history above 1,000,000 records or keys, no multi-segment history, one caller, and a payload of 32 bytes only.
- **Append includes the caller's current submit.** The append phase holds one submit of the caller at a time beside the engine. It is not an engine-only window.
- **This is not a leak test.** The readings establish neither a leak nor the absence of one.

## Checks

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` on the Mac host: 147 unit and scenario tests, 3 process tests and 3 other integration tests. One test is ignored: the scheduled long run.
- Two earlier runs of `cargo test --workspace` did not pass and are not counted. The host's load average was above 90 from other sessions' work, and the scenario deadline aborted the test process. The third run passed. No Rust source differs from `origin/main`.
- `python3 -B -m unittest discover -s scripts -p 'test_m1_*.py'`: 27 script tests on the Mac host and in the Linux container, 15 of them new. They refuse a `status` text without `VmHWM`, one in another unit, one with a repeated field and one that breaks a kernel identity. They refuse a process with a missing barrier, a failed result or an early exit, and a reset that did nothing. They drive a stand-in probe through the barrier protocol against a stand-in `/proc` directory.
- Eight deliberate faults in the driver each fail at least one test: a peak that ignores the start of the phase, a `clear_refs` value of `1`, an accepted missing field, an unchecked barrier order, an unchecked `VmHWM` below `VmRSS`, an unchecked reset, a mean in place of the median, and an unchecked result line.

[checks.txt](checks.txt) holds the result lines.

## Reproduce

Run from the repository root on a Docker host. The script creates the container `seglog-m1-peaks`, builds the probe inside it, runs the script tests, the reset check and the matrix, writes the summary, and removes that container.

```sh
sh docs/evidence/issue20/linux-phase-peaks/run.sh .audit/new-linux-phase-peaks
```

To print the tables again from the raw readings in this record:

```sh
tar -xzf docs/evidence/issue20/linux-phase-peaks/raw.tar.gz -C .audit
python3 scripts/m1-linux-phase-peaks.py summary .audit/trials --csv .audit/readings.csv
```
