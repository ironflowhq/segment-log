# Long-run failure artifacts: hosted drill

[Issue 20](https://github.com/ironflowhq/segment-log/issues/20) section 4 left two points open: no hosted run had shown that a failed long run uploads usable artifacts, and no downloaded artifact had been replayed. No hosted run has failed, so a real failure could not show either. This record forces the failure path on both platforms and replays what it leaves.

## Result

**The failure path works on Linux and macOS.** In [drill run 42754924923](https://github.com/ironflowhq/segment-log/actions/runs/42754924923), both long-run jobs failed on the planted records and uploaded their artifacts. Both replay jobs downloaded them and passed. The same two artifacts, downloaded to a Mac with `gh run download`, replay there with the same result.

**A run without the drill still passes.** [Run 42754928006](https://github.com/ironflowhq/segment-log/actions/runs/42754928006) used the same workflow revision without the input. Both long-run jobs passed with 100 trace seeds and 30 process repetitions, and the replay job was skipped. [plain-run.json](plain-run.json) is its API record.

This shows that a failed check leaves a trace, a ledger and a process directory that can be fetched and checked again. It does not show a real defect, because none was observed.

## What the drill does

`gh workflow run long-run.yml -f drill=true` sets `SEGMENT_LOG_TRACE_DRILL=1` and `SEGMENT_LOG_PROCESS_DRILL=1` for the long-run job. [acceptance-tests.md](../../../acceptance-tests.md#drill-the-failure-path) describes the procedure.

| Step | Planted record | Failure it must cause |
| --- | --- | --- |
| Trace oracle | One `TooLarge` refusal of a request that fits every limit, added to the ledger when it is checked | `false permanent size refusal` |
| Process oracle | A watermark of `u64::MAX` that no child reported | `assertion failed: log.watermark() >= ledger.watermark` |

The planted records exist only in the check. The saved pair and the kept process directory hold what the run recorded. The replay therefore has a control: the plain replay of each artifact must pass, and only the replay with the drill variable must fail, on the planted record.

## Hosted run

Source: `7d8c483b3c27bdc88e621e397aa460aa39114bd2`, branch `t3/m1-failed-artifact-drill`. [run.json](run.json) is the API record and [run.log](run.log) the full log. An earlier drill, [run 42753948712](https://github.com/ironflowhq/segment-log/actions/runs/42753948712) at `3b19599`, gave the same result. The commit between them only renames the replay job.

| Job | Linux (`ubuntu-24.04`) | macOS (`macos-latest`) |
| --- | --- | --- |
| M1 long run | failed, as planted | failed, as planted |
| Trace step | `false permanent size refusal`, seed 10000, `Kill`, variant 0 | the same |
| Process step, which now runs after a trace failure | `log.watermark() >= ledger.watermark`, seed 10000 | the same |
| Uploaded artifact | `trace-failures-ubuntu-24.04`, 10,634 bytes | `trace-failures-macos-latest`, 10,648 bytes |
| Drill replay | passed | passed |

Each replay job first checks that the long-run job failed. It then downloads the artifact of its platform and runs `scripts/m1-drill-replay.sh`. Both jobs end with `drill replay: both plain replays passed and both planted records were rejected`.

The run as a whole ends as failed, because its long-run jobs fail by design. The replay jobs carry the verdict.

## Downloaded artifacts

[artifacts.json](artifacts.json) lists the two artifacts with their digests. Their contents are kept here unchanged:

| Artifact | Trace pair | Process directory |
| --- | --- | --- |
| [ubuntu-24.04](ubuntu-24.04/) | `2223-10000-Kill-0.trace`, `.ledger` | `process/segment-log-slice8-2305-SIGKILL-10000/` with `protocol.txt`, `log/00000000000000000000.seg` and `log/LOCK` |
| [macos-latest](macos-latest/) | `2589-10000-Kill-0.trace`, `.ledger` | `process/segment-log-slice8-2621-SIGKILL-10000/` with the same three files |

## Replay on the Mac

```sh
gh run download 42754924923 --name trace-failures-ubuntu-24.04 --dir .audit/drill-hosted/ubuntu-24.04
gh run download 42754924923 --name trace-failures-macos-latest --dir .audit/drill-hosted/macos-latest
scripts/m1-drill-replay.sh .audit/drill-hosted/ubuntu-24.04
scripts/m1-drill-replay.sh .audit/drill-hosted/macos-latest
```

Both replays exited 0 on the Mac host (Darwin 27.0.0, arm64, Rust 1.99.1) from a clean export of the source commit. Each ran on a copy, so the artifacts kept here are as downloaded. [local-replay-ubuntu-24.04.log](local-replay-ubuntu-24.04.log) and [local-replay-macos-latest.log](local-replay-macos-latest.log) are their output. The Linux artifact replays on macOS, so a pair does not depend on the platform that wrote it.

## Local checks

On the Mac host at the source commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (148 library tests passed, one ignored) and the 12 script tests passed. The new scenario `a_drill_failure_retains_a_pair_that_replays_the_rejection` runs the planted failure and both replays on each pull request.

## Limits

- The drill plants its records in the check. It exercises retention, upload, download and replay. It does not exercise a defect in the engine, the simulator or the oracle.
- The process replay rebuilds the ledger from the transcript and checks the kept log again. The first check already recovered that log. After a real failure its retries may also have appended to it, so a process replay can differ from the first check. The trace replay has no such limit.
- A scheduled execution is a separate proof. The drill does not show that the scheduler starts the workflow.
- Artifacts expire on GitHub under the repository's retention setting. The copies here do not.
