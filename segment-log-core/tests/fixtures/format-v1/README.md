# Format version 1 fixture

The `FORMAT-GOLDEN` scenarios in `src/acceptance/format_golden.rs` pin these files ([ADR 0052](../../../../docs/decisions/0052-freeze-format-version-1.md)).

- `open/`: two rolled segments and an active one, written by the scenario's records with log id `07 07 … 07`, a 300-byte roll size and writer epoch 2.
- `sealed/`: the same log after `seal`.

Never regenerate these files for the same version. Until the first production release, a layout change increments `format_version`, deletes this directory and writes `format-v2/` with the new code, and its pull request states what changed in the bytes ([ADR 0053](../../../../docs/decisions/0053-format-changes-need-no-older-reader-before-production.md)).
