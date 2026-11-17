# Format version 3 fixture

The `FORMAT-GOLDEN` scenarios in `src/acceptance/format_golden.rs` pin these files ([ADR 0064](../../../../docs/decisions/0064-the-0-1-compatibility-promise.md)). Format version 3 is the 0.1 baseline.

The scenario's writer made them with log id `00 01 … 0f`, a 300-byte roll size and the simulated clock. It created the log at epoch 1, with the clock at 2025-11-17T00:00:00Z moving 1 s at each group, and appended three submits. Then it reopened the log at epoch 2, with the clock 2.5 s behind the last group, and appended the rest. The second writer's first three groups therefore carry the first writer's last append time. Segment 0 holds epoch-1 groups and the epoch-2 writer's closing run.

- `open/`: two rolled segments and an active one.
- `sealed/`: the same log after `seal`.

Never regenerate these files. Until the 0.1 tag, a layout change may still replace them under [ADR 0053](../../../../docs/decisions/0053-format-changes-need-no-older-reader-before-production.md). From the 0.1 tag they stay for good: a layout change is a new format version, which adds its own fixture directory beside this one, and every later build must still read these files (ADR 0064, rules 11 and 12).
