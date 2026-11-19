# Larger and longer workloads (issue 47)

[Issue 47](https://github.com/ironflowx/segment-log/issues/47) asks for measurements beyond the M1 sizes (1,000 callers, 1,000,000 records, one segment), and for the operational limits they support. They ran during the M7 requalification on the qualified host's volume drive, with `scripts/m7-workloads.sh` at `82bb5b1` ([raw output](https://github.com/ironflowx/segment-log-evidence/tree/dc1a230a4b0e7bd70fec1963473ddd7cb2733c1c/m7/requalified-host/workloads), [host](../m7/requalified-host/README.md#host)). Each case is the M1 benchmark, `acceptance::evidence::measured_appends_survive_reopen`, at a larger size: keyed 256-byte records, one stream per caller, a unique key per record, default options (1 ms linger, 64 MiB roll), on ext4 on a Samsung PM983.

## Results

| Case | Callers | Records | Segments | Bytes | Append records/s | Append p99 | Reopen | Correctness errors |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| callers | 4,000 | 4,000,000 | 18 | 1.26 GB | 121,851 | 76.5 ms | 178 ms | 0 |
| history | 1,000 | 10,000,000 | 45 | 3.14 GB | 132,523 | 13.9 ms | 40 ms | 0 |
| soak | 1,000 | 60,000,000 | 265 | 18.9 GB | 127,914 | 15.3 ms | 219 ms | 0 |

- Every case reopened with every record and segment it wrote, and the read-back check found every acknowledged offset, revision, key and payload.
- **Throughput holds across rolls and history size.** The soak crossed 264 rolls in 7 min 49 s of appends at 127,914 records/s, the same rate as the 10-million-record history. Across the 325 rolls of the three cases, a roll kept the writer for 0.59 ms at the median, 1.18 ms at p99 and 2.09 ms at most (`ROLL writer_ns` lines in each log).
- **Reopen time stays flat.** 60 million records in 265 segments reopened in 219 ms; sealed segments are read through their footers, not scanned.
- **4,000 callers** ran at about the same rate as 1,000, with no admission refusal (`full=0`). Each caller waits for its answer before it submits again, so the append p99 rises with the caller count: 76.5 ms at 4,000 callers against about 14–15 ms at 1,000.

## Memory

The soak sampled the process's resident set every 5 s ([soak.rss.tsv](https://github.com/ironflowx/segment-log-evidence/blob/dc1a230a4b0e7bd70fec1963473ddd7cb2733c1c/m7/requalified-host/workloads/soak.rss.tsv)).

- **During appends, resident memory grew about 50 bytes per record:** 2.89 GB after about 58 million records. Of that, 16 bytes per record are the harness's own arrays (one latency and one offset per append). So the log's retained state grew at most about 34 bytes per record on this workload.
- The peak of 27.8 GB comes after the appends: the harness's correctness check reads the whole log back into memory in one call. It is not the log's state.

## Operational limits these results support

- One log sustains about 120,000–130,000 keyed records/s on local NVMe in process, from 1,000 to 4,000 callers, across hundreds of rolls.
- A 60-million-record, 19 GB, 265-segment log reopens in well under a second.
- Plan resident memory for the log's retained state at about 34 bytes per record of history for keyed records over few streams, on top of the [idle resident set](../m6/completion.md). A log with many streams retains per-stream state as well; these runs do not measure that case.
- `read(from, max)` returns its records in one allocation; a reader of a large history reads in bounded batches.

## Not measured

- **Several readers with several appenders.** No case runs concurrent readers; the M7 transport tests run one reader per connection. [Issue 156](https://github.com/ironflowx/segment-log/issues/156) tracks it.
- **Many streams in a large history.** Each case uses one stream per caller.
