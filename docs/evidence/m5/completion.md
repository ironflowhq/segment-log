# M5 completion record

M5 (freeze the byte layout, fuzz the recovery parser) passed on 2026-11-13. This record links the evidence for each part of the [M5 gate](../../prd.md#milestones).

| Gate part | Evidence |
| --- | --- |
| A format ADR is Accepted. | [ADR 0052](../../decisions/0052-freeze-format-version-1.md) freezes the layout at `format_version` 1 with no byte change. The owner accepted it on 2026-11-13. The `FORMAT-GOLDEN` scenarios in `segment-log-core/src/acceptance/format_golden.rs` pin the layout against the committed [fixture](../../../segment-log-core/tests/fixtures/format-v1/README.md). |
| Fuzzing runs for 1 hour in CI with no crash. | [Fuzz run 43441567752](https://github.com/ironflowhq/segment-log/actions/runs/43441567752), dispatched at `9874b95` on hosted ubuntu-24.04 runners with `nightly-2026-11-12` and debug assertions on. Each target ran for 3,601 s with no crash: `FUZZ-DECODER` made 362,260,534 runs, and `FUZZ-RECOVERY` made 7,599,542. |

CI is green on Ubuntu and macOS at the same commit, [run 43441514680](https://github.com/ironflowhq/segment-log/actions/runs/43441514680), including the reruns with `roll_bytes` at 65 and 256 bytes.

## Known limit

`FUZZ-RECOVERY` checks that a second open finds the same records only in raw mode. In checksummed mode, the fuzzer found a segment whose footer passes its CRC but names a stream that its groups do not have. The first open takes the stream heads from the groups, and a reopen takes them from the footer. Recovery does not check a footer against the groups it scanned. Building such a footer needs the log id, which no caller has ([ADR 0049](../../decisions/0049-frame-crcs-cover-the-log-id.md)). [testing.md](../../architecture/testing.md#fuzzing-m5) describes the modes.
