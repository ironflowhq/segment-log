# Format version 2 fixture

The `FORMAT-GOLDEN` scenarios in `src/acceptance/format_golden.rs` pin these files ([ADR 0055](../../../../docs/decisions/0055-format-version-2-aligns-key-index-pages.md)).

The scenario's writer made them with log id `00 01 … 0f` and a 300-byte roll size. It created the log at epoch 1 and appended three submits, then reopened it at epoch 2 and appended the rest. Segment 0 holds epoch-1 groups and the epoch-2 writer's closing run.

- `open/`: two rolled segments and an active one.
- `sealed/`: the same log after `seal`.

Never regenerate these files for the same version. Until the first production release, a layout change increments `format_version`, deletes this directory and writes `format-v3/` with the new code, and its pull request states what changed in the bytes ([ADR 0053](../../../../docs/decisions/0053-format-changes-need-no-older-reader-before-production.md)).
