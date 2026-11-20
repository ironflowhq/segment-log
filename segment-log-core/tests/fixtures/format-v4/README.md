# Format version 4 fixture

The `FORMAT-GOLDEN` scenarios in `src/acceptance/format_golden.rs` pin these files ([ADR 0064](../../../../docs/decisions/0064-the-0-1-compatibility-promise.md)). Format version 4 adds the close frame ([ADR 0071](../../../../docs/decisions/0071-a-clean-drop-writes-a-close-frame.md)).

The scenario's writer made them with the same history as the [version 3 fixture](../format-v3/README.md), with a build at version 4. The first writer's clean drop wrote a close frame after its groups in segment 0. The second writer rolled past it, so segment 0 is a sealed segment that holds a close frame between its groups and its closing run.

- `open/`: two rolled segments and an active one.
- `sealed/`: the same log after `seal`.

Never regenerate these files. They stay for good: a layout change is a new format version, which adds its own fixture directory beside this one, and every later build must still read these files (ADR 0064, rules 11 and 12).
