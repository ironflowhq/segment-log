# Several readers, several appenders and many streams (issue 156)

[Issue 156](https://github.com/ironflowx/segment-log/issues/156) asks what the [issue 47 record](../issue47/README.md) left out: readers that run beside appenders on a large history, and the resident memory of a log with many streams.

## Verdict

- **Up to 4 readers showed no cost to appends on this host.** 1,000 appenders ran at 112,874–149,846 records/s (medians) beside 1 or 4 readers of any pattern, with a p99 of 12.6–15.0 ms. With no reader, they ran at 100,762 records/s with a p99 of 16.1 ms. Every case's range overlaps the others, so this run shows no slowdown, and it does not show a speedup either.
- **16 readers that read history compete with the appenders for CPU on this 10-core host.** Beside 16 random or scan readers, appends fell to 83,431–88,385 records/s, with a p99 of 33–38 ms. 16 tail readers, which sleep while they wait, left appends at 103,713 records/s with a p99 of 16.1 ms.
- **Tail and scan reads are cheap. A read at a random offset of a large history is not.** A read of 64 records took 68–112 µs at the median for a tail reader, 170–223 µs for a scan, and 1.5–1.9 ms for a random reader (5.6–5.9 ms with 16 readers).
- **The random-read cost is the frame-table cache.** A log keeps the frame tables of 16 sealed segments ([api.md](../../architecture/api.md)). A read that needs another segment first reads that segment's frame headers again. Random reads stay at about 200 µs up to 7 segments, the p99 jumps at 18, and the median reaches 1.5 ms at 43 ([frame tables](#frame-tables)). Scans do not change, because they stay in one segment for many reads.
- **A stream costs about 140–230 bytes of resident memory.** That is with 100,000 or 1,000,000 streams with 8-byte names, measured after appends and after recovery.

These are diagnostics from a Mac's internal SSD. They are not a target claim, and the operational limits for a deployment need a rerun on the qualified host.

## Host and provenance

- 2026-11-19, Apple M4 Mac mini: 10 cores, 24 GiB, Darwin 27.2.0, APFS on the internal SSD (Apple Fabric). rustc 1.100.0, release build.
- Measured at `e935e1b` with the changes to `readers_probe.rs`, `run.sh` and `summarize.py` that this change commits: a tail reader that sleeps, a shuffled case order, a fixed history and the scripted frame-table check.
- The host ran an interactive session but no other benchmark. The raw output was not kept, per the [evidence rules](../README.md). To reproduce it, run [run.sh](run.sh): `docs/evidence/issue156/run.sh OUT_DIR 10000000 20 3` (about 35 minutes).

## Method

[run.sh](run.sh) drives every part, and [summarize.py](summarize.py) prints the tables below from its output.

**Readers.** `readers_probe build` writes 10,000,000 keyed 256-byte records over 1,000 streams under the default options, in groups of 256 records: 3.0 GB in 43 segments, in 25.8 s. Every case then runs on its own copy-on-write clone of that log, so each case starts at the same watermark. A case reopens the clone and runs for 20 s:

- 1,000 closed-loop appenders, each sending one keyed 256-byte record per submit on its own new stream, as in the M1 benchmark;
- beside them, 0, 1, 4 or 16 readers that call `read(from, 64)`:
  - `tail` follows the watermark, and sleeps 200 µs when it has read everything durable;
  - `random` reads at a uniform offset of the 10,000,000-record history;
  - `scan` reads the history in order, from evenly spaced starting points.

Every read checks that its records are contiguous and carry 256-byte payloads. One discarded warm-up case runs first. Then 3 rounds run every case once, each in an order shuffled with the round number as seed.

`LogReader` opens only a sealed log, so it cannot run beside appenders on the same log (api.md). It was measured alone: `readers_probe seal` seals one more clone, and `readers_probe sealed` runs the random and scan readers through `LogReader` on it.

**Frame tables.** `run.sh` builds and seals logs of 1,500,000, 4,000,000 and 10,000,000 records (7, 18 and 43 segments). One reader then reads each through `LogReader` for 10 s, at random and in order, 3 times.

**Memory.** [streams.py](streams.py) runs the existing `resident_probe` on 1,000,000 records with 32-byte payloads and 8-byte stream names, without keys, in groups of 1,000. It uses 1, 1,000, 100,000 and 1,000,000 streams. It runs an append process, then a recover process on the log that process wrote, and reads the resident set with `ps` at the probe's barriers. It does this 3 times for each count.

## Appends beside readers (1,000 appenders)

| Readers | Pattern | Runs | Records/s median (range) | p50 µs | p99 µs | p999 µs | Full |
| ---: | --- | ---: | --- | ---: | ---: | ---: | ---: |
| 0 | - | 3 | 100,762 (100,456–135,319) | 9,658 | 16,114 | 22,733 | 0 |
| 1 | random | 3 | 112,874 (112,357–158,638) | 8,642 | 15,020 | 21,282 | 0 |
| 1 | scan | 3 | 130,591 (128,020–176,859) | 7,869 | 12,583 | 16,865 | 0 |
| 1 | tail | 3 | 130,616 (129,475–136,756) | 7,056 | 14,010 | 23,610 | 0 |
| 4 | random | 3 | 144,866 (110,516–145,673) | 6,610 | 14,001 | 21,016 | 0 |
| 4 | scan | 3 | 126,651 (120,177–158,883) | 8,084 | 12,988 | 19,599 | 0 |
| 4 | tail | 3 | 149,846 (107,569–169,191) | 6,444 | 12,853 | 20,820 | 0 |
| 16 | random | 3 | 83,431 (82,082–92,949) | 10,448 | 33,070 | 50,743 | 0 |
| 16 | scan | 3 | 88,385 (87,225–91,339) | 9,796 | 37,986 | 63,954 | 0 |
| 16 | tail | 3 | 103,713 (99,040–110,005) | 9,716 | 16,125 | 22,357 | 0 |

The baseline is below every case with 1 or 4 readers. It was again in a first run of this record, which had a fixed case order. Shuffling the order did not remove that gap. But the runs of one case differ by up to 57 %, and the ranges of the cases with 0 to 4 readers overlap, so 3 runs cannot separate them. The cause of the spread is not known.

## Reads (64 records per call)

| Log | Readers | Pattern | Runs | Records/s median (range) | p50 µs | p99 µs | p999 µs | max µs |
| --- | ---: | --- | ---: | --- | ---: | ---: | ---: | ---: |
| live | 1 | random | 3 | 47,004 (42,013–47,800) | 1,585 | 3,304 | 27,623 | 44,227 |
| live | 1 | scan | 3 | 342,678 (303,243–347,553) | 176 | 707 | 1,870 | 28,562 |
| live | 1 | tail | 3 | 130,578 (129,425–136,697) | 71 | 899 | 1,662 | 5,055 |
| live | 4 | random | 3 | 163,450 (161,003–171,299) | 1,961 | 3,958 | 7,923 | 52,996 |
| live | 4 | scan | 3 | 1,233,156 (1,195,277–1,286,227) | 190 | 946 | 2,047 | 43,692 |
| live | 4 | tail | 3 | 599,186 (430,076–676,566) | 68 | 653 | 1,642 | 6,056 |
| live | 16 | random | 3 | 141,731 (140,966–142,999) | 5,857 | 33,436 | 54,230 | 99,553 |
| live | 16 | scan | 3 | 1,809,467 (1,763,703–1,839,843) | 223 | 6,984 | 15,202 | 77,544 |
| live | 16 | tail | 3 | 1,658,602 (1,583,843–1,759,217) | 112 | 1,059 | 2,220 | 5,870 |
| sealed | 1 | random | 3 | 63,266 (62,622–63,291) | 1,483 | 1,688 | 1,762 | 3,125 |
| sealed | 1 | scan | 3 | 407,228 (407,046–407,279) | 170 | 208 | 227 | 1,792 |
| sealed | 4 | random | 3 | 201,272 (195,469–202,293) | 1,862 | 2,153 | 4,033 | 6,657 |
| sealed | 4 | scan | 3 | 1,455,949 (1,446,715–1,470,709) | 187 | 317 | 424 | 2,212 |
| sealed | 16 | random | 3 | 155,317 (154,292–155,835) | 5,635 | 29,674 | 41,800 | 129,713 |
| sealed | 16 | scan | 3 | 2,066,085 (2,033,442–2,072,411) | 220 | 7,128 | 17,801 | 49,334 |

- A tail reader reads at the append rate, because it can read only what is durable. Each of 4 tail readers reads every record, so together they read 4 times the append rate.
- The appenders add about 10 segments in each case, but random readers read only the 10,000,000-record history, so every case reads at random over the same 43 segments.

## Frame tables

| Records | Segments | Pattern | Runs | Records/s median (range) | p50 µs | p99 µs | p999 µs |
| ---: | ---: | --- | ---: | --- | ---: | ---: | ---: |
| 1,500,000 | 7 | random | 3 | 349,115 (348,320–349,541) | 206 | 293 | 309 |
| 1,500,000 | 7 | scan | 3 | 396,273 (396,239–396,407) | 184 | 216 | 234 |
| 4,000,000 | 18 | random | 3 | 238,752 (238,629–239,165) | 216 | 1,628 | 1,702 |
| 4,000,000 | 18 | scan | 3 | 394,188 (393,985–394,323) | 187 | 214 | 226 |
| 10,000,000 | 43 | random | 3 | 63,198 (63,167–63,442) | 1,466 | 1,678 | 1,763 |
| 10,000,000 | 43 | scan | 3 | 394,398 (392,907–394,596) | 187 | 217 | 235 |

At 18 segments, a random read misses the 16 cached tables about 1 time in 9, and the p99 shows it. At 43, most reads miss, and the median shows it. A miss costs more when a segment holds more frames. In the first run of this record, the history grew to 314 segments written by 1,000 single-record callers, so each segment held more, smaller groups, and a random read took 4–6 ms at the median with 1–4 readers.

## Resident set by stream count

Median KiB over 3 runs, for 1,000,000 records without keys, with 8-byte stream names.

| Streams | After open | After appends | After recovery | Appended − open | Recovered − baseline |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 2,640 | 3,536 | 2,768 | 896 | 800 |
| 1,000 | 2,640 | 3,872 | 2,992 | 1,232 | 1,024 |
| 100,000 | 2,640 | 25,792 | 24,688 | 23,152 | 22,720 |
| 1,000,000 | 2,640 | 178,592 | 141,824 | 175,952 | 139,856 |

After subtracting the 1-stream row, 100,000 streams cost 228 bytes each after appends and 225 after recovery. 1,000,000 streams cost 179 and 142 bytes each. The cost per stream varies with the stream count, likely because hash tables grow in steps and so are fuller at some counts than others. This run did not check that. Plan for about 230 bytes per stream with short names, plus the name's length. This adds to the per-record figure for keys in the issue 47 record.

## Limits of this run

- 3 runs per case cannot separate the append rates of the cases with 0 to 4 readers (see above).
- A Mac on APFS is not the deployment stack. Its flush, its scheduler and its page cache differ from the qualified Linux host. Clones of one history may also share cached pages on APFS, which favours reads of history.
- `ps` reports the resident set, not the bytes the engine owns. The [issue 48 record](../issue48/README.md) separates those.

## Follow-up

A reader that reads at random over more than 16 sealed segments pays to read a segment's frame headers on nearly every call. To keep more tables cached, or to make a miss cheaper, is a design change that needs its own issue, and an ADR if it changes the format.
