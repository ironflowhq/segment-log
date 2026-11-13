# Architecture decisions

Record architecture decisions as ADRs in this folder, one file per decision, named `NNNN-short-title.md` (e.g. `0001-segment-file-format.md`). Numbers are sequential and never reused. Number 0000 is the template.

Read the relevant ADRs before changing code they cover. When you propose a design choice that would contradict an accepted ADR, say which one and argue against it in a new ADR rather than silently diverging.

To write an ADR, copy [0000-template.md](0000-template.md) to the next free number and fill in every section. Quote `id`, `supersedes` and `superseded_by` (`"0006"`), because YAML can read a number with a leading zero as octal.

To reverse a decision, write a new ADR. Set its `supersedes` to the old number. In the old ADR, set `status` to `superseded` and `superseded_by` to the new number. Don't edit the old decision away.

A later ADR can amend a decision without replacing it. Then add a bullet at the top of the old ADR's Consequences that links the new ADR. When a PRD open item can change a decision, say so in the same place.

After an ADR is accepted, its Decision and Options stay as accepted. A pull request may correct its Enforcement section, for example to name the test that implements it, if the change keeps what the enforcement checks. Add a bullet at the top of Consequences that names the pull request and says what changed. Any other change needs a new ADR.
