# Exact phase peaks and engine-owned bytes for issue 48

[Issue 48](https://github.com/ironflowhq/segment-log/issues/48) asks for three things that the [Linux phase peaks record](../issue20/linux-phase-peaks/README.md) left open. First, the exact peak of each phase: engine, caller workload, validation, read and reopen. Second, the bytes that the engine owns, apart from the caller and the test. Third, a statement of how exact each figure is, checked against a buffer of known size.

This record adds a driver that replays the kernel's own trace of every change to a process's resident set. It adds a probe that gives the engine its own allocator arena, and it adds tests for both. It changes no production code, default, API or format. The probe adds two dev-dependencies, `talc` and `libc`. `segment-log-core` still uses only `std`.

## Result

**Exact peaks: in 502 of 504 phases, the peak is the larger of the phase's two ends.** The matrix has 122 traced processes: four histories, five probe modes and three repetitions, each run once unpinned and once pinned to one processor, plus one control process for each. The driver replays 1,515,242 kernel events. At each of the 628 barriers, the replay ends at the exact `VmRSS` that the kernel prints. No trace instance lost an event. Two phases peak above both of their ends:

- One pinned engine drop of the 16,384-record history peaks 192 KiB above its start. The kernel's mark recorded that peak.
- One unpinned append of the 1,000,000-record keyed history peaks 24 KiB above its end. The mark missed all of it.

So the earlier record's reading holds, now as an exact result instead of a lower bound: each phase peaks at its own steady state. The largest transient seen is 192 KiB.

Exact peak of each phase, in KiB above the same process's baseline. A cell is the median [range] of six processes, three unpinned and three pinned. One number alone means that all six gave it.

| Phase, and the barrier that ends it | 16,384 records | 16,384 records, all keyed | 1,000,000 records | 1,000,000 records, all keyed |
| --- | ---: | ---: | ---: | ---: |
| Caller workload: build every submit, no engine (`caller_held`) | 2,692 | 3,204 | 164,104 | 195,352 |
| Engine: open a new log (`empty_held`) | 380 | 380 | 380 | 380 |
| Engine: append every group (`appended_held`) | 464 | 2,198 [2,196–2,200] | 974 [972–976] | 101,516 [101,508–101,520] |
| Read: one record, after the append (`read_one_held`) | 478 [476–480] | 2,202 [2,196–2,208] | 974 [972–976] | 101,566 [101,548–101,572] |
| Reopen: recover the history, in a new process (`recovered_held`) | 424 | 2,140 | 700 | 101,176 |
| Read: one record, after the reopen (`read_one_held`) | 424 | 2,140 | 700 | 101,176 |
| Validation: read every record, after the reopen (`validation_held`) | 3,104 | 5,332 | 164,828 [164,828–166,120] | 296,530 [296,528–296,532] |

**The control proves that the replay sees a transient.** The `pulse` mode allocates, fills and drops a 16,384 KiB buffer inside one phase, so neither barrier holds it. In both control processes the replay finds a peak 16,388 KiB above the larger of the phase's two ends: the buffer plus one header page from glibc. Unpinned, the kernel's mark also saw the full peak. Pinned, it read 64 KiB low.

**Pinning reduces the mark's error but does not remove it.** The mark is below the exact peak in 69 of 253 unpinned phases, by 12–436 KiB, and in 67 of 253 pinned phases, by 36–188 KiB. It is never above the exact peak. In the phases where the resident set fell by more than 1 MiB, the median error is 188 KiB unpinned and 122 KiB pinned. The Linux 7.0 source explains this (see [How exact each figure is](#how-exact-each-figure-is)): the mark comes from per-processor counters that can each lag by 31 pages for each kind of page. Pinning removes the other processors' lag, not the lag of the one that the process runs on.

**Engine-owned bytes.** `owner_probe` sends every allocation that engine code requests to a separate arena. That is every allocation on a thread other than the main thread, and every allocation on the main thread inside an engine call. Everything else goes to the system allocator. The probe counts requested bytes in each allocator. It subtracts the engine's results that the caller holds, and it reads the resident pages of the arena and of the writer's stack from the page tables. The table gives the engine's bytes at chosen barriers, in KiB. Rows after open and append come from the three append processes, rows after reopen from the three recover processes. A cell is the same in all three unless it shows a median [range]. [summary.md](summary.md) has every barrier.

| Engine bytes at the barrier | 16,384 records | 16,384 records, all keyed | 1,000,000 records | 1,000,000 records, all keyed |
| --- | ---: | ---: | ---: | ---: |
| Requested bytes after open (`empty_held`) | 289.7 | 289.7 | 289.7 | 289.7 |
| Requested bytes after every append (`appended_held`) | 292.8 | 1,603.1 | 438.7 [412.5–449.2] | 76,279.2 [76,278.8–76,295.1] |
| Requested bytes after reopen (`recovered_held`) | 292.6 | 1,603.0 | 412.3 | 76,251.9 |
| Arena resident after reopen (`recovered_held`) | 324 | 2,100 | 516 | 103,800 |
| Writer stack resident, at every barrier after open | 12 | 12 | 12 | 12 |
| Peak requested bytes in the validation read, after reopen | 2,371.3 | 3,811.6 | 129,735.4 | 213,395.3 |
| The validation result that the caller then holds | 2,048 | 2,176 | 129,174.5 | 136,987 |

- **Keys.** Keyed minus unkeyed requested bytes after reopen is 1,310.4 KiB for 16,384 keys and 75,839.6 KiB for 1,000,000 keys. That is 81.9 and 77.7 bytes for each key, the figures that the [key positions record](../issue20/key-positions/README.md) counted on an earlier commit. In the arena the same differences are 1,776 and 103,284 KiB resident, or 111.0 and 105.8 bytes for each key, with talc's overhead and recovery's freed scratch included.
- **Read.** In the validation read, the engine's peak exceeds the bytes that it holds plus the finished result by 30.6–32.5 KiB with 16,384 records and 148.5–156.3 KiB with 1,000,000. A one-record read peaks 28.4–30.5 KiB above the held bytes with 16,384 records and 109.8–117.7 KiB with 1,000,000.
- **Append.** During the append phase the engine's peak is 34.7–36.8 KiB above the median bytes that it holds at the end with 16,384 records, and 175.6–182.6 KiB with 1,000,000 records.
- **Nothing outlives the engine.** After the engine is dropped, the arena holds no live bytes in any process. The caller and the test hold 1.8 KiB at every barrier where they hold no workload.

Under glibc the same engines add 380 KiB to the process at open, against 289.7 KiB requested and 12 KiB of stack. The difference is glibc's own overhead and is not attributed further.

## How exact each figure is

| Figure | Exact? | Why, and how it was checked |
| --- | --- | --- |
| Steady resident set (`VmRSS`) | Exact, to the page | The kernel sums its per-processor counters for `status` (`get_mm_counter_sum` in `fs/proc/task_mmu.c`). |
| Phase peak from the replay | Exact, to the page, with one limit below | Each `rss_stat` event carries the exact new value of the counter that changed (`percpu_counter_sum_positive` in `include/trace/events/kmem.h`). The replay ended at `VmRSS` at all 628 barriers, so no change went untraced. No instance lost an event. The pulse control replayed as its known size plus one page. |
| Kernel mark (`VmHWM`) | Low by up to 436 KiB unpinned and 188 KiB pinned here | `update_hiwater_rss` and `get_mm_hiwater_rss` read `get_mm_counter`, which is `percpu_counter_read_positive` (`include/linux/mm.h`). The batch is `max(32, 2 × processors)` = 32 here (`lib/percpu_counter.c`), so each processor can hold up to 31 pages of each counter. Pinned, the bound is 31 pages for anonymous and 31 for file pages: 248 KiB. |
| Engine requested bytes | Exact, by construction | The probe's allocator counts every request. A 1 MiB buffer counted exactly 1,048,576 bytes in each allocator, and exactly zero after its drop, in all 24 owner processes. |
| Arena resident bytes | Exact page count | The probe reads `/proc/self/pagemap` over the arena. A separate 1 MiB mapping counted exactly 256 pages when touched and none after `munmap`, in all 24 owner processes. |
| Writer stack resident bytes | Exact page count | `Rss` of the mapping that holds an address on the writer's stack, from `/proc/self/smaps`, which walks the page tables. |

The replay has one limit. A thread updates a counter and then emits its event. If another thread changes the same counter in between, the first event already includes the second change. A peak that lasts only that long can be missed. Pinning narrows this to a preemption between the update and the event, because this kernel is preemptible. Every phase in the matrix ends with the replay equal to `VmRSS`, so a missed state can only be an interval inside a phase, never a count.

The source lines are from Linux `v7.0`. The guest runs `7.0.14-linuxkit`, and this record did not inspect the vendor's patches. The `rss_stat` format in [environment.txt](environment.txt) matches the upstream tracepoint.

## What "engine-owned" means here

- **Engine code** is every thread other than the main thread, plus the main thread inside `open`, `append`, `wait`, `read`, `watermark` and the drop of the log. The probe spawns no threads of its own, so the writer is the only other thread.
- **A block belongs to the allocator that made it**, whoever frees or grows it. A submit that the caller builds stays in the system allocator while the engine queues it. The engine clones the keys and stream ids that it keeps (`Keys::stage` in `engine/keys.rs`), so no caller block stays in the engine after the submit is answered. At every barrier, no submit is in flight.
- **Results that the engine returns** are built in the arena. The probe counts the heap bytes of the returned records from their capacities, checks that every block lies in the arena, and subtracts them as bytes that the caller holds. A result still under construction counts as the engine's, so a read's peak includes its result.
- **Arena resident pages are a high-water mark.** The arena never returns memory to the operating system. Its pages are the most that the engine's allocations have touched since the process started, under talc's placement, not glibc's. The arena opts out of transparent huge pages. With them, a touched byte would count 2 MiB.

## Scope and provenance

- The base is `origin/main` at `619d13e2ce4f77ea05d8bae4461a6af022ce9831`. The work tree held this record's changes beside it ([sources.txt](sources.txt)). [source-hashes.txt](source-hashes.txt) gives the hash of every changed source file as it ran, and [binaries.sha256](binaries.sha256) gives the hashes of both probe binaries and the drivers.
- Captured on 10 November 2026 at 18:06 UTC in the pinned `rust:1.99.1-bookworm` image (`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`) on Docker Desktop for Mac. The guest runs Linux 7.0.14-linuxkit on aarch64 with 10 processors, 4 KiB pages, glibc 2.36 and transparent huge pages set to `always` ([environment.txt](environment.txt), [docker-info.txt](docker-info.txt)). The host is an Apple M4 with 24 GiB on macOS 27.0.1 ([host.txt](host.txt)).
- The container is privileged and shares the host's process ids (`--pid=host`), because the trace names processes by those ids. Pinned processes ran on processor 9.
- Other containers ran in the same guest ([docker-stats-before.txt](docker-stats-before.txt), [docker-stats-after.txt](docker-stats-after.txt)). The run was not gated on load: a page count does not depend on timing, and the remote trace instance would record any change that another context made to a probe's memory. It recorded none.
- [run.sh](run.sh) holds the exact commands. It ran once and took 51 seconds. Before it, pilots ran in a separate container: a hand-written replay of three processes, and two driver pilots with one repetition of the two smaller histories. The second pilot found that the owner probe's 1 MiB calibration buffer lent the engine resident arena pages. The calibration moved out of the arena before this run.
- The full event traces are 10 MB of compressed text and are not in the repository. [traces.sha256](traces.sha256) lists the hash of each. [raw.tar.gz](raw.tar.gz) holds one JSON file for each process: its command, every line it printed, every raw `/proc` text, and each phase's replay result with its start, end and peak values.
- This is a virtual Linux guest on a Mac. It is not target hardware.

## Method: exact peaks

`resident_probe` stops at a barrier at the end of each phase and waits on stdin. It gains one mode, `pulse`, the known transient. `scripts/m1-exact-peaks.py peaks` runs each probe process under two `tracefs` instances of the `kmem:rss_stat` event:

1. **Own.** `set_event_pid` names the probe, and `event-fork` adds its writer thread when the engine spawns it. Every event must come from one memory and from a thread whose memory it is.
2. **Remote.** All events with `curr == 0`: a change made from another context, such as reclaim or `khugepaged`. The driver keeps those for the probe's memory and merges them by time. Both instances use the `mono` clock.

At the first barrier the driver reads `status`, starts the replay from its exact counters and enables the trace. At each later barrier it takes a consistent snapshot: it drains both instances, reads `status`, and drains again, and repeats the read until no event arrived around it. Then it replays the phase's events in order, keeps the largest resident sum, and refuses the process unless the replay ends at `VmRSS`. It also reads `smaps_rollup`, keeps the mark, and resets the mark with `clear_refs` as the earlier record did. After each process it refuses any instance that overwrote or dropped an event.

The four histories are those of the earlier record: 16,384 records in groups of 256 on one stream, with no keys and with 16,384 keys, and 1,000,000 records in groups of 1,000 on 1,000 streams, with no keys and with 1,000,000 keys. Payloads have 32 bytes, and keys and stream ids have eight. Linger is zero, and every other option keeps its default. Each repetition runs the `touch`, `caller`, `empty`, `append` and `recover` modes of each history in fresh processes. Recovery reopens the log that the append process of the same repetition wrote. The whole matrix runs unpinned, then pinned with `taskset`, and the driver checks the pinned processes' `Cpus_allowed_list`.

[summary.md](summary.md) has every barrier of every mode, with the steady resident set, the exact peak, the peak above both ends and the mark's error, pinned and unpinned. [readings.csv](readings.csv) has one row for each barrier of each process.

## Method: engine-owned bytes

`owner_probe` maps a 2 GiB region with `MAP_NORESERVE` and hands it to one `talc` allocator behind a mutex. Its global allocator sends each request to the arena or to the system allocator as described above, and returns each block to the allocator whose range holds it. Two counters per allocator keep the live requested bytes and the peak since the last barrier. The probe prints one row at each barrier. The row has live and peak bytes for the engine and for everything else, the bytes of returned results that the caller holds, the arena's present pages from `pagemap`, and the writer stack's `Rss`.

The known-size checks never touch the arena before the engine is gone. A 1 MiB separate mapping checks the page count right after the baseline. A 1 MiB buffer in each allocator checks the counters after the engine is dropped. `scripts/m1-exact-peaks.py owners` runs `append`, then `recover` on the same directory, for each history and repetition. It refuses a process whose checks are not exact, whose rows do not add up, or whose engine bytes outlive the engine.

## Limits

- **The replay's peak is exact to the page, with the one concurrency limit above.** It is a whole-process figure, like the earlier record's steady states.
- **Engine figures depend on the arena.** Requested bytes do not depend on any allocator. Arena resident pages are talc's high-water mark, not what glibc would keep resident. Under glibc, engine pages share a heap with the caller's pages, which is why the earlier record could not attribute them.
- **Freed memory changes later steady states.** In two of the three pinned 1,000,000-record keyed append processes, dropping the engine returned about 100,000 KiB to the system. No unpinned process did. That is allocator behavior, not an instrument error, and it shows that a steady state after a drop says little about the engine.
- **The trace runs beside the probe.** It does not change any count. It slows each page fault, which can change the timing between the writer and the caller.
- **One guest, warm cache, three repetitions, four histories.** No history above 1,000,000 records or keys, no multi-segment history, one caller, and 32-byte payloads only. The numbers depend on glibc 2.36 malloc, 4 KiB pages and transparent huge pages set to `always`. 36 barriers had huge pages, all holding a control buffer or a validation result of 1,000,000 records.
- **No PRD target is decided.** The PRD's idle resident set target is for the M6 server binary.
- **This is not a leak test.** The probe checks only that no engine bytes outlive the engine.

## Checks

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` on the Mac host, on the tree that ran: 152 unit and scenario tests, 3 process tests and 3 other integration tests passed. One test is ignored: the scheduled long run.
- `python3 -B -m unittest discover -s scripts -p 'test_m1_*.py'`: 53 script tests in the Linux container, 18 of them new ([checks.txt](checks.txt)). They refuse a lost-events marker, a thread that changes another memory, a replay that ends away from `VmRSS`, lost events, a pin that did not hold, a pulse that the replay misses, an owner check that is not exact, held bytes that do not add up, and engine bytes that outlive the engine. They drive a stand-in probe through the barrier protocol against a stand-in `/proc` and trace.
- [faults.py](faults.py) applies 12 deliberate faults to the driver, one at a time. Each one fails at least one test ([checks.txt](checks.txt)): a peak that ignores the phase's start, a snapshot that trusts the first read, accepted lost events, an accepted replay mismatch, remote events of another memory kept, own events of another memory accepted, own events from another context accepted, skipped unknown trace lines, swap counted as resident, a dropped lower bound on the pulse, an unchecked pin, and skipped owner checks.

## Reproduce

Run from the repository root on a Docker host whose kernel has `tracefs` and the `kmem:rss_stat` event. The script creates the privileged container `seglog-m1-exact`, builds both probes inside it, runs the script tests and the fault check, runs both matrices, writes the summary, and removes the container.

```sh
sh docs/evidence/issue48/run.sh .audit/new-issue48
```

To print the tables again from the raw readings in this record:

```sh
mkdir -p .audit/issue48-raw
tar -xzf docs/evidence/issue48/raw.tar.gz -C .audit/issue48-raw
python3 scripts/m1-exact-peaks.py summary .audit/issue48-raw --csv .audit/issue48-raw/readings.csv
```
