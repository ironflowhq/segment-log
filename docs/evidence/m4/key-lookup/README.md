# Retained-key lookup prototype

This folder holds the measurements behind [ADR 0048](../../../decisions/0048-retained-keys-use-a-log-filter-and-segment-key-files.md). The prototype is throwaway code. It is not part of the log and is not built by CI.

## What it measures

The prototype generates random keys of 16 to 36 bytes for 100 sealed segments, at two scales:

- S1: 200,000 keys per segment, 20 million keys in all. This is about one keyed record of 330 bytes per entry in a 64 MiB segment.
- S2: 20,000 keys per segment, 2 million keys in all.

For each candidate in the ADR, it measures open time, resident memory, the writer's cost to look up a new key, the cost of a retry lookup, the work added to a roll, and rebuild cost. Each timing repeats 5 to 7 times. The results report the median, the minimum and the maximum. The candidate names in the result files map to the ADR as follows.

| Result name | ADR option |
| --- | --- |
| `A0` | Exact map of full keys, as the code holds today |
| `A` | Exact map of 64-bit key hashes, loaded from key files |
| `B` | Option B, one filter for each segment |
| `C4`, `C16`, `C64` | Option C, one log filter of 4, 16 or 64 MiB |
| `D4`, `D16`, `D64` | Option D, the log filter in front of the segment filters |

## Conditions and limits

- macOS on Apple silicon, local APFS disk, warm page cache. The target platform is Linux on NVMe, so these numbers are diagnostics, not target results.
- `run.sh` waits for a low host load before each run, and each run records a CRC throughput check at its start and end.
- `coldtest.txt` compares one random 4 KiB read from cache (about 0.75 µs) with one from the device (about 76 µs). A design that reads the device on the path of every new key cannot meet the append target.
- The cold-cache runs and the repeat runs were stopped for time. Within-run repeats give the spread.
- The files `*-p512-*` use 512-byte key-file pages instead of 4 KiB pages. Page size changes only the retry lookup.

## Rerun

```sh
cd source
cargo build --release
./run.sh x S1 4096 > S1.txt
./run.sh x S2 4096 > S2.txt
```

The runs write about 1 GiB of test data under `source/data`.
