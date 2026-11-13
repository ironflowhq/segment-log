# Format version 1 fixture

The `FORMAT-GOLDEN` scenarios in `src/acceptance/format_golden.rs` pin these files ([ADR 0052](../../../../docs/decisions/0052-freeze-format-version-1.md)).

- `open/`: two rolled segments and an active one, written by the scenario's records with log id `07 07 … 07`, a 300-byte roll size and writer epoch 2.
- `sealed/`: the same log after `seal`.

Never regenerate these files. A layout change increments `format_version` and adds a new fixture directory beside this one. These files must still open.
