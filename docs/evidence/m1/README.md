# M1 implementation evidence

This archive preserves selected implementation evidence from the slice PRs. The
[provenance table](provenance.tsv) identifies original files, extraction methods
and limitations. Original paths in captured output describe the development
environment. They are not dependencies of the current tests.
The provenance notes identify whitespace normalization where applied; original
byte hashes remain recorded for those files.

The [acceptance workflow](../../acceptance-tests.md) and
[completion gate](../../architecture/testing.md#m1-completion-gate) own the
requirements. This archive does not establish M1 completion.
Outstanding work is tracked in [issue #17](https://github.com/ironflowhq/segment-log/issues/17).

| Slice | PR | Observed evidence and limits |
| --- | --- | --- |
| 0 | [#6](https://github.com/ironflowhq/segment-log/pull/6) | [Contemporaneous journal](provenance/slice0.tsv) and [parent corrections](provenance/parent-decisions.tsv). Several initial behaviors were implemented before their scenarios. Original raw red output is unavailable. Narrative entries cannot repair that history. |
| 1 | [#7](https://github.com/ironflowhq/segment-log/pull/7) | [Command journal](provenance/slice1.tsv), captured `red/slice1-*.txt` outputs, [counter journal](provenance/counters.tsv) and five counter red logs. Early admission failures did not independently exercise later fault assertions. |
| 2 | [#8](https://github.com/ironflowhq/segment-log/pull/8) | Seven supplemental scenarios. The experimental red assumed two unflushed groups, an unreachable premise under [ADR 0015](../../decisions/0015-open-makes-the-recovered-tail-durable.md). Its experimental fix was reverted; it is not claimed as a production defect. |
| 3 | [#9](https://github.com/ironflowhq/segment-log/pull/9) | Seven supplemental revision scenarios; no production fix or behavioral red. Compilation and fixture corrections are excluded from red evidence. |
| 4 | [#10](https://github.com/ironflowhq/segment-log/pull/10) | [Seven-test red run](red/slice4-durable-keys.log), extracted from the original output column. The separate short-frame guard correction has [narrative evidence](provenance/parent-decisions.tsv), with no retained raw stdout. |
| 5 | [#11](https://github.com/ironflowhq/segment-log/pull/11) | [Five-test pending-key red run](red/slice5-pending-keys.log). The earlier fixture-timeout run is excluded. |
| 6 | [#12](https://github.com/ironflowhq/segment-log/pull/12) | Fourteen supplemental scenarios. [Wrapping-add mutation rejection](mutation/slice6-overflow.log) is sensitivity evidence, not a discovered production defect. The temporary mutation changed the final `charge_base` addition from checked to wrapping arithmetic. |
| 7 | [#13](https://github.com/ironflowhq/segment-log/pull/13) | Five delegate scenarios reached 121 tests. Review added an admission-close observation and sixth scenario, reaching 122. The final [parent record](provenance/parent-decisions.tsv) preserves this distinction. |
| 8 | [#15](https://github.com/ironflowhq/segment-log/pull/15) | Seven supplemental scenarios and a reproduced harness scheduling failure. [Final workspace summaries](validation/slice8-workspace-summary.log) show 125 core, three process and one real-storage test. [Thirty additional repetitions](validation/slice8-process30.log) used seeds 5001–5030. |

Slice 9 adds the [threaded trace oracle and coverage audit](slice9.md), including
saved replay commands, the preserved pressure-seed regression and local validation.
Its scheduled Linux/macOS workflow is configured but has not run here.

## Reproduce the process-harness finding

Run from the repository root with Python 3, `patch` and the pinned Rust toolchain:

```sh
python3 docs/evidence/m1/process-schedule/verify.py --controlled-repetitions 20
```

The runner builds disposable test crates against the current core. It expects the
preserved pre-gate harness to fail, then expects the current harness to pass a
controlled schedule. It writes results under ignored `.audit/process-evidence/`
without editing repository sources. The preserved final source was commit
`4bf4844` when these patches were recorded.

The preserved [pipe transcript](process-schedule/protocol.txt) contains 250
requests and 250 answers, no request 0 and zero unknown outcomes. The
[failure output](process-schedule/observed-red-output.txt) is explicitly a
transcription of a captured tool result. The original stdout was not redirected
to a file. The [schedule patch](process-schedule/schedule-only.patch) changed only
caller scheduling and parent event consumption.

Fresh review retained [100 ordinary repetitions](process-schedule/fixed-ordinary.log)
and [20 controlled repetitions](process-schedule/fixed-controlled.log), seeds
30000 onward. The controlled schedule asserts that the former kill predicate is
reached before the post-admission abandonment event. The final gate waits for
that event. No production code changed for this harness correction.

## Validation environment and outstanding evidence

The slice 8 local executions used Rust 1.99.1 (`58cf8f9c3`, LLVM 23.1.1), host
`aarch64-apple-darwin`, macOS 27.0.1 build 26A434. Formatting, strict workspace
Clippy, workspace tests, case-ID validation and diff checks passed. The fuzzing
configuration compiled with 24 existing unused simulator warnings.

Slice 8 CI passed on [Linux](https://github.com/ironflowhq/segment-log/actions/runs/42559576442/job/128972810634)
and [macOS](https://github.com/ironflowhq/segment-log/actions/runs/42559576442/job/128972811118)
at commit `4bf4844`. The [captured check metadata](validation/slice8-ci.json)
identifies the head and completion times.

The randomized oracle and scheduled-job configuration are recorded in
[slice 9](slice9.md). Hosted execution of that new workflow is not claimed.
Slice 10 submits [completion evidence](slice10.md), including actual Linux
syscalls, Linux and macOS directory calls, and repeated Linux guest benchmarks.
The record distinguishes target deviations, unavailable physical Linux NVMe
results and missing physical-stack qualification. Historical missing initial
reds remain disclosed. The owner still closes the M1 gate.
