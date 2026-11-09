# Read allocation and retained-history investigation

The goal is to investigate issue [#20](https://github.com/ironflowhq/segment-log/issues/20), compare alternatives, and make a focused M1 improvement supported by runtime evidence. This result supports bounding the frame snapshot for a read. It does not close #20 or decide the M1 gate in #17.

## Result

PASS for reduced requested heap allocation. A one-record read at the start of 512 single-record groups allocated 21,207 bytes before and 767 bytes after. Each value repeated exactly across five alternating runs. At the end, the corresponding values were 21,143 and 703 bytes. All 120 probe runs validated every record and reported zero failures. These are instrumented debug-build allocation counts, not latency, throughput, RSS, or hardware qualification measurements.

The difference is exactly `(512 - 1) * 40 = 20,440` bytes of unused frame descriptors. The change uses binary searches over ordered frame bounds and clones only intersecting frames. It releases the publication mutex before storage reads, as before. It preserves a single publication snapshot and still validates and decodes every selected group in full.

## Source and environment

- Merged PR #19 baseline is `911e809e72fc9c9ef5571f7022c89e55a48d6cb1`.
- Probe and boundary-test source is `09392d2778a032fc5290ea8b50299cdbbb32bd34`. The library at that commit is identical to the merged baseline.
- Selected implementation and experiment runner are `04e7bd628ab85a6d2ae27053b07d73ea5fb6624a`. Documentation added afterward does not change the measured implementation.
- The suffix candidate is the probe commit plus [suffix.patch](candidates/suffix.patch). It was built and run, then replaced by the selected implementation.
- Captured on November 9, 2026, on Apple M4, Mac16,10, 10 logical CPUs, 24 GiB RAM, macOS 27.0.1 build 26A434, aarch64-apple-darwin. Data was in the local worktree under `.audit/memory/data` on `/Volumes/p7`.
- Rust 1.99.1, compiler commit `58cf8f9c35b355199727627d07ae34a4e2846d67`, LLVM 23.1.1. `cargo build` used the default dev profile. No new dependencies.
- The initial host load averages were 143.77, 61.81, and 28.59. Only small allocation and correctness runs ran here. No timing, RSS, cold-cache, physical NVMe, or VPS qualification claim is made.

The saved binary SHA-256 hashes identify the exact local artifacts. The raw output stays ignored under `.audit/memory`.

| Artifact | SHA-256 |
| --- | --- |
| baseline | `4dd5943ac1eab0a1fa32d2c13d4c072c640aa90285fc6873d53c070a4fbdce96` |
| suffix | `70241390e99f27056916c96947fba20b0ea3799a2d0f509c49bf236caf689940` |
| bounded | `9555ac42848dd3ad664408d37ff3799430c1daeecc026dbe16289664059b8587` |
| raw comparison.csv | `7d1ed10e8bf76bb36ec5c033811ffa45d45e55207e2ca6c799b4889a8356fbe7` |

## Method

The probe uses the public real-storage engine and wraps the system allocator in the example executable. The library's unsafe-code prohibition is unchanged. Allocation counters do not allocate. Thread-local counters count all requested allocation bytes on the calling thread, including the full requested size of reallocations. Atomic counters track live requested bytes across all Rust threads and the extra peak above the start of each phase. Freed bytes reduce the live counter. Allocator size classes, native thread stacks, libc allocations, file cache, and kernel memory are excluded.

Per-phase process counters can include asynchronous writer startup or cleanup. Their ranges are retained in [allocations.csv](allocations.csv). They must not be interpreted as exact ownership or resident memory. Reader-thread allocation totals were identical in all five repetitions of every case and candidate. The first read has a consistent additional 64-byte allocation in all candidates. The comparison retains this first-use effect.

The phases distinguish these costs:

- `open_empty` establishes the engine and its fixed queue overhead.
- `caller_build` retains all input submits before admission. Caller history is separate from the engine's admission budget.
- `append_and_release_caller` consumes those submits and waits for every commit. Its thread counter omits writer-thread allocations. Its process counters include them.
- `drop_before_reopen` joins the writer and releases the first engine.
- `warm_reopen` scans and validates existing groups, rebuilds indexes, rewrites and syncs the tail, and starts the writer. Its requested bytes are not a scan-time measurement.
- `read_start` and `read_end` each return one record and retain it until after the counters are sampled.
- `validation_buffer` reads the full history into a retained result. `validation_check` verifies offset, revision, stream, key, and all payload bytes without allocation. `validation_release` drops that buffer.
- `drop_engine` releases the reopened engine. The amount freed includes its indexes and queue. It is a useful retained-heap comparison, not a proof of steady-state RSS.

Every submit is awaited before the next, and linger is explicitly zero. Other options retain their defaults. These diagnostic settings make group count deterministic. Group count in the output is derived from the successful sequential submits, not an independent on-disk frame census. Record and commit assertions fail the process on any error. The runner also checks the final validation count and success marker. There are no rejected or retried requests in the successful runs.

## Independent history dimensions

Every key and stream identifier has eight bytes. Unique keys appear on the first K records. Streams cycle round-robin. Payload bytes encode the record index modulo 251. Five repetitions ran baseline, suffix, bounded in that order for each case. This alternation holds the dataset and path constant between candidates.

| Case | Records | Keys | Streams | Records per group | Groups | Payload bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| base | 512 | 0 | 1 | 32 | 16 | 32 |
| records_quarter | 128 | 0 | 1 | 8 | 16 | 32 |
| keys_half | 512 | 256 | 1 | 32 | 16 | 32 |
| keys_all | 512 | 512 | 1 | 32 | 16 | 32 |
| streams_64 | 512 | 0 | 64 | 32 | 16 | 32 |
| groups_64 | 512 | 0 | 1 | 8 | 64 | 32 |
| groups_per_record | 512 | 0 | 1 | 1 | 512 | 32 |
| payload_256 | 512 | 0 | 1 | 32 | 16 | 256 |

## What grows and what remains

These are medians for the selected implementation. Full ranges for every candidate and phase are in the CSV.

| Case | Heap bytes freed by engine Drop | Caller-build live delta bytes | Validation-buffer live delta bytes | One-record start allocation bytes |
| --- | ---: | ---: | ---: | ---: |
| records_quarter | 265199 | 16970 | 16384 | 1943 |
| base | 265199 | 66122 | 65536 | 5975 |
| keys_half | 312847 | 68170 | 67584 | 6495 |
| keys_all | 360399 | 70218 | 69632 | 6495 |
| streams_64 | 269903 | 66122 | 65536 | 5975 |
| groups_64 | 267119 | 67274 | 65536 | 1943 |
| groups_per_record | 285039 | 78026 | 65536 | 767 |
| payload_256 | 265199 | 180810 | 180224 | 20535 |

At fixed key, stream, and group counts, more records or payload bytes did not increase the observed heap released by engine Drop. More unique keys, streams, or groups did. This agrees with the ownership model in `Published.frames`, `Published.heads`, and `Keys`. The 512-key case released 95,200 more bytes than the unkeyed base case. That difference describes this B-tree occupancy and key size, not a universal per-key constant.

Large groups still cost more to read even for a one-record result. `codec::decode` builds a borrowed-entry vector for the entire group. The file read and CRC scratch each allocate group-sized byte buffers. Raising payload size from 32 to 256 bytes increased the bounded start read from 5,975 to 20,535 requested bytes. This change deliberately leaves decoding and CRC unchanged.

Reopen allocation traffic also grows. With 16 unkeyed groups and one stream, 128 records requested 291,307 bytes during reopen and 512 records requested 358,891 bytes, despite identical observed retained engine heap. Recovery scans and validates all groups. None of these runs bounds reopen time for a production history.

The admission budget does not bound retained indexes, caller buffers, reader allocations, or process memory. M1 retains one descriptor per group plus key and stream maps. No leak is established by these results. Durable indexes, caches, roll, and footer changes remain outside this fix.

## Candidate selection

1. Baseline clones all descriptors and scans past earlier groups. It costs 21,207 bytes for the start read in the 512-group case.
2. The suffix candidate binary-searches the starting frame and clones the remaining history. End reads fall to 703 bytes, but start reads stay at 21,207 bytes. Reject because it leaves unrelated future history in the snapshot.
3. The selected candidate binary-searches both interval endpoints. Start reads fall to 767 bytes and end reads to 703 bytes. `saturating_add` preserves oversized-limit behavior. No shared-state layout, write-path, public API, dependency, or durable-format change is needed.

Keeping the publication lock throughout decoding would avoid the snapshot allocation but would hold it during file I/O and CRC work. That alternative was not implemented or measured. A shared immutable metadata tree would add a new write-path structure. Neither is needed for the measured allocation reduction.

## Reproduce

Use disposable source archives to build the exact candidates with the pinned toolchain. Run these commands from the repository root. Each destination must be new.

```sh
mkdir -p .audit/memory .audit/memory-baseline .audit/memory-suffix
git archive 09392d2778a032fc5290ea8b50299cdbbb32bd34 | tar -x -C .audit/memory-baseline
git archive 09392d2778a032fc5290ea8b50299cdbbb32bd34 | tar -x -C .audit/memory-suffix
patch -d .audit/memory-suffix -p1 < docs/evidence/m1-memory/candidates/suffix.patch
cargo build --manifest-path .audit/memory-baseline/Cargo.toml -p segment-log-core --example memory_probe
cargo build --manifest-path .audit/memory-suffix/Cargo.toml -p segment-log-core --example memory_probe
cargo build -p segment-log-core --example memory_probe
python3 scripts/m1-memory-probe.py \
  baseline=.audit/memory-baseline/target/debug/examples/memory_probe \
  suffix=.audit/memory-suffix/target/debug/examples/memory_probe \
  bounded=target/debug/examples/memory_probe \
  --data .audit/memory/data --output .audit/memory/comparison.csv
```

The captured run used identical binary contents copied to `.audit/memory/baseline`, `.audit/memory/suffix`, and `.audit/memory/bounded`. The full original command was:

```sh
python3 scripts/m1-memory-probe.py baseline=.audit/memory/baseline suffix=.audit/memory/suffix bounded=.audit/memory/bounded --data .audit/memory/data --output .audit/memory/comparison.csv
```

`allocations.csv` groups the raw CSV by variant, case, and phase and takes min, integer median, and max of each metric across five runs. No samples were removed. A longer history can be requested with `--records`, subject to the unchanged group-byte and submission limits. Separate larger RSS and timing experiments remain pending.

## Verification and limits

[regression.txt](regression.txt) preserves an actual baseline failure of a 4,096-byte one-record allocation ceiling, followed by the selected candidate's pass at the same path and dimensions. This is an allocation regression check, not a previously failing correctness contract.

The new public integration test passed both before and after the implementation. It checks empty history, zero limit, offsets beyond the head, oversized limits that saturate, unequal multi-record groups, reads crossing group boundaries, reopen, and 64 rounds of reads with concurrent appends. Each read must contain an exact contiguous prefix of one of the two permitted publication states. It does not claim every possible thread schedule.

Recorded commands at implementation commit `04e7bd628ab85a6d2ae27053b07d73ea5fb6624a`:

```sh
cargo test -p segment-log-core --test read_ranges
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

All passed. Workspace results were 135 unit tests, three process tests, one read-range test, and one real-storage test, with one scheduled long-run unit test ignored. Existing recovery, crash, corruption, and I/O-failure coverage passed. No correctness or durability failure was observed. Independent root review is pending. Earlier M1 missing initial-red evidence remains missing; this supplemental result does not repair that history.
