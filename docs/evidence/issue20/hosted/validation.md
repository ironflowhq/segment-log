# Local validation on 9 November 2026

Source: `64cc15dc250c855b28c0d012465c3e3780a382cc`.
Commands ran from the repository root. The selected logs are direct captures;
the strict check before the change used baseline
`911e809e72fc9c9ef5571f7022c89e55a48d6cb1`.

| Command | Exit | Observation |
| --- | --- | --- |
| `RUSTFLAGS='--cfg fuzzing' cargo check -p segment-log-core` at baseline | 0 | 24 dead-code warnings, all in the private simulator |
| `RUSTFLAGS='--cfg fuzzing -D warnings' cargo check -p segment-log-core` at baseline | 101 | 24 dead-code errors, [captured output](fuzz-strict-before.log) |
| Same strict check after the correction | 0 | No warnings, [captured output](fuzz-strict-after.log) |
| `cargo fmt --all --check` | 0 | No output |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | Both workspace crates checked without warnings |
| `python3 -B -m unittest discover -s scripts -p 'test_m1_*.py'` | 0 | 3 tests passed |
| `cargo test --workspace` | 101 | Core test binary aborted with SIGABRT after the codec test reported FAILED; [captured output](workspace-aborted.log) |
| `cargo test -p segment-log-core --lib acceptance::codec::binary_groups_round_trip_and_encoding_enforces_limits -- --exact --nocapture` | 101 | Isolated binary also aborted with SIGABRT; no assertion diagnostic, [captured output](codec-isolated-aborted.log) |
| `cargo test -p segment-log-core --lib storage:: -- --nocapture` | 0 | 23 storage and simulator tests passed, 113 filtered out |

The workspace run did not reach a complete test summary. It is not a pass.
The isolated failure did not expose an assertion mismatch. The test uses
`bounded`, which creates a 10-second deadline whose worker aborts the process.
At isolation time, `uptime` reported load averages `221.59 134.61 65.26`, and
`ps -Ao pid,%cpu,comm` showed many concurrent `yes` processes. This supports a
resource-contention explanation, but no unloaded rerun established the cause.
The source correction changes a gate that is already true for tests, so the
test configuration still includes exactly the same simulator code.

No workload, watchdog, or other worker's processes were changed to obtain a
pass. Full workspace validation remains unresolved and needs a rerun after the
concurrent stress workload finishes. The targeted simulator tests show that the
selected gate retains executable simulator coverage despite this limitation.

Deslop review found only the one-line gate change in executable source. No
comment, lint suppression, test timeout, API, or default changed. The requested
no-subagents boundary prevents a separate comment-review agent; self-review
found no added code comments, and the coordinator retains independent review.
