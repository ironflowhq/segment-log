---
id: "0064"
title: "From 0.1, no release breaks an older log or an older client"
status: accepted
date: 2026-11-17
deciders: [Altug Sahin]
supersedes: "0053"
superseded_by: null
tags: [format, protocol, compatibility, release]
touches: [docs/architecture/format.md, docs/architecture/protocol.md, docs/architecture/api.md, docs/architecture/testing.md, docs/prd.md, segment-log-core/src/codec/**, segment-log-core/src/engine/**, segment-log-core/tests/fixtures/**, segment-log-server/src/**, clients/go/**, Cargo.toml, .github/workflows/ci.yml]
enforced_by: ci
---

# 0064. From 0.1, no release breaks an older log or an older client

## Context

The owner declared on 2026-11-17 that 0.1 is the first production release ([issue 133](https://github.com/ironflowx/segment-log/issues/133)). [ADR 0053](0053-format-changes-need-no-older-reader-before-production.md) lets every format change break older logs until that release and requires an ADR that states the production promise before it. [ADR 0057](0057-a-hand-rolled-binary-protocol-answered-in-order.md) does the same for the wire protocol. The PRD's risk table asks for the same ADR.

These facts are fixed:

- **The protocol forces lockstep upgrades.** `Hello` carries one `version`, and a server accepts only its own (protocol.md, Session). An unknown `kind` is malformed, and a nonzero reserved field is malformed. So any change to the bytes, even an added request kind, breaks every deployed client at once.
- **Clients and servers upgrade independently.** The server runs in a guest image ([ADR 0059](0059-m7-qualifies-firecracker-on-a-dedicated-nvme-host.md)) and its callers run on the host, released separately. A rollout replaces guests one at a time while callers keep running.
- **A build reads only its own `format_version`** (format.md, Status; `segment-log-core/src/engine/survey.rs`). The version lives in each segment's header, and the only state shared across segments is the `log_id`, offset continuity and non-decreasing epochs. None of these depends on the version, so one log can hold segments of different versions.
- **Every counter is already 64 bits** on disk and on the wire: offsets, revisions and epochs. Counts and lengths are 32 bits or less, bounded by limits far below their range.
- **A survey of planned features** (2026-11-17) found 25 candidates. Only the repair tool ([issue 1](https://github.com/ironflowx/segment-log/issues/1)) is in 0.1, and it changes neither the protocol nor the format. Every deferred feature can be expressed as a new request kind, a new field, a new status code or a new frame kind. None needs a wider field or a rewrite of old data. Retention, time-based seek and debugging all want an append time, and the format has none.
- **A format review** (2026-11-17) found no field that forces a rewrite, but found gaps: footer entry order is unstated though ADR 0054 requires byte equality; the epoch rules are split between format.md and overview.md; limits are not tied to their version; and a key index page CRC covers only the page bytes.
- **Cargo treats 0.x specially:** `0.1 → 0.2` may break the API, and `0.1.0 → 0.1.1` must not. Go treats v0 modules as unstable.

## Options & trade-offs

### Option A - Freeze today's bytes and promise nothing more than exact versions

- **For:** no work before 0.1.
- **Against:** every later addition to the protocol is a lockstep upgrade of guests and callers, which Context rules out. A format change after 0.1 would either break 0.1 logs or need a reader design that nobody wrote down. Data written by 0.1 never gets an append time.

### Option B - Add extension points before 0.1, then promise reads and negotiation across every 0.x release

- **For:** after 0.1, features arrive as additive versions: a new protocol version that older clients never negotiate, and a new format version written one segment at a time. No deployment upgrades in lockstep, and no log is rewritten. The changes are cheap now, because ADR 0053 still allows breaks.
- **Against:** one more pre-production break of both the format and the protocol. Each build keeps decoders for every older format version and encoders for older closing runs, and each server keeps every older protocol version until a major release.

### Option C - Add speculative features before the freeze

Examples: request ids, a log selector, an authentication exchange, record headers, feature flags.

- **For:** fewer versions later.
- **Against:** it freezes guesses. Most of these have no decision behind them, and B makes each one an additive version when a decision needs it.

### Option D - Migrate logs with a tool instead of reading old versions

- **For:** a build reads only one version.
- **Against:** an operator must stop and rewrite every log at each format change. Mixed-version reading costs less and needs no downtime.

## Decision

We will use option B. Rules 1–4 are the last pre-production breaks, made under ADR 0053 before the 0.1 tag. Rules 5–16 take effect at the 0.1 tag, and this ADR then supersedes ADR 0053.

**Before 0.1: the last breaks.**

1. **Protocol version 2 negotiates a range.** The `Hello` request is `magic` (4), `min_version` (2), `max_version` (2). The success answer is status, chosen `version` (2), `frame_cap` (4), reserved (2). The server chooses the highest version both sides speak. With no overlap, it answers `UnsupportedVersion` with its own `min` (2) and `max` (2), then closes. This layout of `Hello` never changes again.
2. **Format version 3 adds an append time and binds index pages.** Each group frame carries a u64 wall-clock append time in milliseconds since the Unix epoch, written by the writer when it seals the group. The time never decreases within a log: the writer stamps the larger of the clock and the previous group's time, and open takes the previous time from the newest group it recovers, so a clock that steps back cannot reorder appends. The key index page CRC is seeded with the segment's `log_id`, `base_offset` and the page number. Record headers stay out.
3. **format.md states the rules that the code and ADRs already depend on:**
   - footer entries are sorted by stream id bytes, compared as unsigned bytes;
   - every frame of a closing run carries the run's epoch;
   - a frame's epoch is never lower than its segment header's epoch;
   - every limit and constant (frame body limit, group cap, roll limit, page size, filter parameters) belongs to its format version, so changing one is a version bump.
4. **Versions keep counting.** Format version 3 and protocol version 2 are the 0.1 baseline. No version resets to 1, so a pre-production log or client is refused with `UnsupportedVersion`, never misread.

**From 0.1: the protocol.**

5. A server speaks every protocol version from 2 up to its own. Removing a version needs an ADR and a major release. A major release is 1.0, then 2.0 and so on: no 0.x release removes a protocol or format version.
6. Capabilities follow the negotiated version, with no feature flags. Modes that depend on configuration, such as `--read-only`, show as status codes, or later through an info request kind.
7. A `kind` outside the negotiated version is malformed. Reserved fields stay written as zero, and a nonzero one is malformed. A later version may give a reserved field a meaning only on connections that negotiated it.
8. A new status code either comes only from a request kind that older versions lack, or names an existing code that the server sends to connections on older versions.
9. Any handshake beyond `Hello`, such as a log selector or authentication, is a new request kind in a later version, sent after `Hello`.
10. The protocol version counts byte changes only. The changelog maps each release to the protocol versions it speaks.

**From 0.1: the format.**

11. A build reads every format version from 3 up to its own. Dropping a version needs an ADR, a major release and a migration path.
12. A new format version arrives one segment at a time. A reader checks each segment against its own version. A writer never appends to an older-version segment. At open, it closes an older active segment in that segment's own version, then rolls into the new version. An older active segment that holds only its header has that header rewritten in place, because a roll would reuse its file name. The rewrite is one write of the whole header (64 bytes, inside one sector), then `sync_all` of the segment and of the directory; it relies on V2's sector atomicity, as ADR 0015's rewrite does, and the crash simulation covers a crash before, during and after it. Sealed segments are never rewritten. So a build keeps the encoders for older versions' closing runs (roll, recovery and seal), not only their decoders.
13. `magic`, `format_version` and `header_len` keep their offsets in every version. The header CRC sits at `header_len - 4`.
14. An upgrade is one-way. An older build refuses a newer segment with `UnsupportedVersion`. Every release that bumps the format says so in the changelog, because rolling the binary back is safe only until the new build writes.

**From 0.1: the APIs.**

15. `segment-log-core`, `segment-log-server` and the Go client follow 0.x semver. API breaks go only in minor releases, each in the changelog with a migration note, and patch releases never break. The server's flags, exit codes and documented process behavior are API: a removal or rename is warned for one minor release first. The Go client is tagged `clients/go/vX.Y.Z` in step with the crates.
16. The crates declare `rust-version`. It rises only in a minor release and is tested in CI.

Rules 5–14 hold across minor releases. A 0.2 that breaks the Rust API still reads 0.1 logs and still serves 0.1 clients.

B beat A because A turns every future feature into a coordinated outage, and beat C because C freezes guesses. We accept, on purpose, that each build keeps every older format version's decoders and closing-run encoders, and every older protocol version, until a major release.

## Consequences

- Accepted by the owner on 2026-11-17 for the 0.1 release ([issue 133](https://github.com/ironflowx/segment-log/issues/133)), with three clarifications made before acceptance: a major release is 1.0 and later (rule 5), the append time never decreases within a log (rule 2), and the header rewrite of rule 12 is one sector-sized write covered by crash cases.
- ADR 0053 is superseded at the 0.1 tag. Until then it governs rules 1–4. ADR 0052's rule "a build reads every frozen version from 1 up to its own" returns in rule 11, starting at version 3. ADR 0057's "the server accepts one version" ends with rule 5.
- protocol.md and format.md change their Status lines, and change their bytes for rules 1–3, in the implementation pull request. `FORMAT-GOLDEN` gets a `format-v3` fixture.
- Fixtures can no longer be replaced after 0.1. `format-v3/` and the protocol version 2 fixture stay for good, and each later version adds a fixture beside them.
- The writer needs a wall clock. The golden fixtures and ADR 0054's byte-equal rebuild of closing runs need an injectable clock, as `Fault::Random` already gives for the `log_id`. The rebuild must reproduce the stored times, not read the clock again.
- Every later format version costs a reader, the older closing-run encoders and mixed-version tests. Every later protocol version costs conformance runs at each supported version.
- The repair tool (issue 1) must handle segments of every supported version.
- Releases need a `CHANGELOG.md` that lists API breaks, format bumps and protocol versions.
- PRD open item 11 (bounded history) gains a note: deleting segments needs a cumulative revision checkpoint, which a later format version can add at roll.

## Enforcement

The implementation pull request adds:

- `FORMAT-GOLDEN` for each supported format version, plus a mixed-version scenario: a log written at version N is opened and appended to by a build at version N+1. The scenario uses a test-only version bump until a real version 4 exists.
- A golden protocol fixture per supported version (`Hello`, and one message of each kind), plus the shared conformance cases run at every version the server claims, in CI.
- A negotiation test: overlapping ranges choose the highest common version; disjoint ranges answer `UnsupportedVersion` with the server's range.
- `cargo-semver-checks` against the last release tag, in CI.
- A CI job that builds and tests at `rust-version`.

Removing a supported version fails the golden and conformance jobs. That is the trigger for the ADR and the major release.

## Revisit if

- A feature needs a change that no additive version can express, such as a wider field or a rewrite of old segments.
- The cost of keeping old protocol versions or old closing-run encoders becomes a measurable burden. Then plan a major release.
- A deployment needs to roll back across a format bump after the new build has written. Then reconsider a mode that keeps writing the older format.
- A consumer needs per-record metadata. Then record headers get their own format version and decide whether they count in the key match ([api.md](../architecture/api.md)).
