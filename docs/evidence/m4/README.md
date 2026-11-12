# M4 gate measurements

The M4 row of the [milestone table](../../prd.md#milestones) asks for a Linux report that gives open time cold and warm, key-cache memory, and the effect on append throughput and p99 latency. [testing.md](../../architecture/testing.md#benchmarks-m1-for-the-core-m6-for-the-server) adds two conditions: every record is keyed, and the appends run beside the same workload on the M3 code. [ADR 0048](../../decisions/0048-sealed-segments-carry-a-key-filter-and-index.md) gives the memory formula. The [prototype](key-lookup/README.md) that chose the key filter is a separate record.

Three files produce the report:

- `segment-log-core/examples/sealed_probe.rs` builds a fixture, opens it, or runs appends on it. It uses only the public API, which is the same in the M3 code and in M4.
- `scripts/m4-evidence.sh` builds the probe for both sides, runs every measurement, and records the environment.
- `scripts/m4-summary.py` writes `summary.md` and fails on any run that reported an error.

## Capture on a hosted runner

Dispatch the `m4-evidence` workflow, or put the `m4-evidence` label on a pull request. A dispatch can set `sealed` and `roll_bytes` to measure a named variant. The workflow measures the pull request's head, or the dispatched ref, against the M3 code at `0ffc772`. It uploads the `m4-evidence-linux` artifact, which GitHub keeps for 3 days. A capture writes about 20 GB of fixtures under `/mnt`. The job's time limit is 150 minutes. No full capture has run yet, so its real duration is unknown.

To capture on another Linux host, run this from the repository root:

```sh
SEGMENT_LOG_EVIDENCE_DIR=/path/on/the/disk/under/test scripts/m4-evidence.sh OUT_DIR [HEAD [BASE]]
```

Cold opens need root, either directly or through passwordless `sudo`. Without root, the script skips the cold runs and writes the reason to `open/cold-not-applicable.txt`. `M4_COLD=off` skips them on purpose.

`M4_EVIDENCE_SMALL=1` shrinks every size so that the tooling runs in under a minute: 3 sealed segments of 1 MiB, 20 callers, 2 pairs. Use it to check the scripts, and never report its numbers. The `M4_SEALED`, `M4_ROLL_BYTES`, `M4_ACTIVE_BYTES`, `M4_OPEN_RUNS`, `M4_CALLERS`, `M4_ROUNDS`, `M4_PAIRS` and `M4_INTERVAL_US` variables set each size one at a time. Report a changed size as a named variant ([ADR 0036](../../decisions/0036-measure-the-existing-defaults-before-tuning.md)).

## The fixture matches the PRD open-time condition

The [target](../../prd.md#footprint-and-performance-targets) is 50 ms with a 4 MiB active segment and 100 sealed segments with footers. The sealed segments use the 64 MiB default `roll_bytes` from [api.md](../../architecture/api.md), as ADR 0048 does when it counts about 20 million keys.

The probe appends 1,000-record submits through one caller, with linger set to zero. Each record has a 256-byte payload, an 8-byte stream id and a 16-byte key, which makes a 296-byte record entry. Each submit names each of 1,000 streams once. The probe stops when 100 segments are sealed and the active segment holds at least 4 MiB. It counts records per segment from the segment file names, so the report states measured counts, not these predictions:

- Each group frame is 296,032 bytes. A segment rolls after the group that reaches 64 MiB, so a sealed segment holds 227 groups, or 227,000 keyed records.
- The 100 sealed segments hold 22,700,000 keyed records. The active segment holds 15,000 records in 4,440,544 bytes. That is at least 4 MiB and less than one group frame over it.
- The key filters take `8 * ceil(n / 255) + 64 * ceil(n / 32)` bytes for each sealed segment with `n` keys (ADR 0048 rule 5, format.md). That is 461,144 bytes per segment and 46,114,400 bytes in all, about 2.03 bytes per key.

The unkeyed fixture has the same roll size, stream count and stop rule, with no keys. Its 280-byte entries put 240,000 records in each sealed segment. Its footers are the same size as the keyed fixture's footers. With an open that reads only footers and filters, the gap between the two open times is the cost of the filters. With an open that scans records, the unkeyed fixture also has 6% more records to scan.

## The M3 comparison builds one probe source on both sides

The M3 code cannot gain new code, and its in-library benchmark has no setting for the roll size or for sealed history. The script therefore copies this checkout's `sealed_probe.rs` into a worktree of each side and builds it there. Both binaries run the same workload source. Only the library under it differs.

Each side builds its own keyed fixture, because the M3 code does not read M4 key frames. Each run starts from that fixture. It hard-links the sealed segments, copies the active segment, then appends 1,000 callers × 1,000 rounds of keyed single-record submits. That is 1,000,000 new keys per run, which roll about four more segments. Every new key misses, so M4 asks each sealed segment's filter, newest first, and M3 asks its in-memory map. The closed loop measures throughput. A second closed loop, `closed-unkeyed`, sends the same records without keys to the same keyed fixture. It skips the key lookup and keeps everything else, so the gap between the two closed loops on one side is what the lookup costs on that side ([ADR 0051](../../decisions/0051-a-log-key-filter-in-memory-in-front-of-the-segment-filters.md)). The paced run, at 20 ms per caller, offers 50,000 records/s and measures the latency target ([ADR 0045](../../decisions/0045-latency-target-is-measured-at-a-paced-rate.md)). Runs alternate, before then after, for 5 pairs of each of the three modes, after one warm-up per side. At the end, the script checks that no run changed a fixture file.

## Output files

| File | Contents |
| --- | --- |
| `environment.txt` | Date, head, base, probe hash, sizes, cold-run status, toolchain, kernel, OS, CPUs, memory, mount options of the data disk, block devices, load |
| `build-before.log`, `build-after.log`, `binaries.sha256` | Probe builds for each side and their hashes |
| `fixture-<side>-<keyed\|unkeyed>.log` | `BUILD` time, `FIXTURE` shape (records, sealed segments, records per sealed segment, active bytes, directory bytes) and `FILTERS` (sealed keys, expected filter bytes, key index bytes) |
| `open/<keyed\|unkeyed>-<cold\|warm>.log` | One `OPEN` line for each run: time, watermark, heap held after open, peak heap, resident set before and after, page cache before, check failures. Then `OPEN_SUMMARY` with min, p50 and max |
| `open/cold-not-applicable.txt` | The reason that the cold runs did not run, if they did not |
| `warmup-<side>.log` | The unreported warm-up run of each side |
| `append/<closed\|closed-unkeyed\|paced>/<side>-<n>.log` | `CONFIG` with every active option, `OPEN` time, `APPEND` throughput, refusals and check failures, `METRIC append_ns` with p50, p99 and p999, then `/usr/bin/time -v` output |
| `append/<closed\|closed-unkeyed\|paced>/load-<side>-<n>.txt` | Load and processor idleness before each run |
| `fixture-<side>-keyed-changes.txt` | Empty when no run changed that fixture |
| `summary.md` | Tables of all of the above, with the median and range for each side |
| `exit-status.txt` | The script's exit status |

## Read the numbers with their limits

A hosted runner is a shared virtual machine. Its disk differs from run to run, and its root mount uses `nobarrier`. Record these as the conditions, and do not read a target pass from one capture. Before you compare a gap with the target, compare it with the range of its runs. A gap inside the range is no measurable difference.

- **Open time.** Each run is `SegmentLog::open` from call to return, with the log already closed. Warm runs follow the cold runs and find the files in the page cache. Cold runs drop the page cache before each open. `page_cache_before_kib` shows the drop, but the device and the hypervisor can still cache data. Open time depends on the code under test. An open that scans every record grows with the 7 GB of history, and an open that reads only footers and filters reads about 46 MB.
- **Key-cache memory.** The probe counts the bytes that the global allocator holds after open. The keyed fixture's count minus the unkeyed fixture's count is the measured key-cache memory. The summary compares it with the filter bytes from the formula. The rest is the active segment's exact map and the index-page fences. The heap count excludes allocator overhead and freed pages that the allocator keeps. The resident set includes both, so its change is larger and noisier.
- **Append throughput and p99.** The closed loop is bound by one `fdatasync` for each group and the 1 ms linger. A difference between the sides appears only when lookups or rolls add time on the writer thread. The M3 side opens by reading every record, so its open takes seconds and holds every key in memory. That open is outside the timed appends, and `Max RSS KiB` in the summary shows its memory. The two sides' fixtures differ by the key frames, about 4.1 MB per sealed segment on the M4 side.

Numbers from macOS show only that the tooling works. The targets are for Linux.
