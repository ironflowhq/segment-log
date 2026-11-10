# A group's linger from its first submit's admission: append evidence

[Issue 20](https://github.com/ironflowhq/segment-log/issues/20) left the paced p99 above the PRD bound by 0.13 to 0.25 ms. [ADR 0046](../../../decisions/0046-linger-runs-from-the-first-submits-admission.md) proposes to count a group's linger from the admission of its first submit. This record measures that change against the tree before it.

## Result

**Paced at 50,000 records/s, the latency target's load.** The p50 fell in every one of 20 alternating pairs, on a 4-processor hosted Linux runner and on a 10-processor Docker guest. The medians are 1.52–1.65 ms before and 1.17–1.18 ms after on the hosted runner, and 1.28 ms before and 1.02 ms after on the guest. A request waits 0.25 ms less at the median for its group to stop gathering.

**The p99 fell where slow flushes did not set it.** Its median fell in all three hosted comparisons: 4.51 to 3.65 ms, 3.51 to 3.31 ms and 5.03 to 4.12 ms. On the guest, the quietest run before the change misses the bound by 0.13 ms, and the quietest run after it is 0.40 ms inside. Runs with slow device flushes have a p99 of 2.4 to 39 ms on either side.

**The closed loop costs up to 5%.** The hosted runner shows no established change in throughput. The guest shows 5% less: 434,600 to 412,835 records/s, with separated ranges. Its p50 rose from 2.05 to 2.41 ms and its p99 fell from 3.97 to 3.50 ms. Both figures are eight times the throughput target.

**The writer flushes 35 to 49% more often at the paced rate.** A group closes after each linger, not after each linger plus a flush.

This is evidence from virtual machines. It does not show that a PRD target passes or fails on target hardware. In these comparisons the p99 was within the bound of the same run in 1 of 20 runs before and 3 of 20 after. The [M1 completion record](../../m1/completion.md) has the capture of the final tree.

## Scope and provenance

- "Before" is the tree without the change and "after" is the tree with it. Both include the benchmark additions of the same pull-request stack.
- Hosted comparisons 1 and 2: `c5d3ed03` against `dd4b9d18`. Hosted comparison 3 and the first version below used earlier commits of the same two branches. Each `environment.txt` names its commits, and each `binaries.sha256` identifies the binaries.
- Hosted runner: GitHub `ubuntu-24.04`, AMD EPYC 7763, 4 processors, 16.8 GB, Linux 7.0.0-1017-azure. The data was on ext4 on `/dev/root`, mounted with `nobarrier`, so `fdatasync` sends no cache flush to the device there. Runs [42763514037](https://github.com/ironflowhq/segment-log/actions/runs/42763514037), [42763537029](https://github.com/ironflowhq/segment-log/actions/runs/42763537029) and [42759874347](https://github.com/ironflowhq/segment-log/actions/runs/42759874347).
- Docker guest: the pinned `rust:1.99.1-bookworm` image on Docker Desktop for Mac, Linux 7.0.14-linuxkit, aarch64, 10 processors, 8.3 GB, data on the container's overlay. The host is an Apple M4. [docker/gated.sh](docker/gated.sh) waited before each run until the other containers used less than 10% of a processor and the host's other processes less than two processors. `pairs.txt` in each directory holds what it saw.
- The Mac was saturated by unrelated work for most of 10 November 2026. Guest runs from that time are not in this record: group flushes took 2 to 3 ms there in place of 0.3 ms. The guest comparison ran in a quiet window.
- None of this is local NVMe or a deployment storage stack.

## Method

Every run uses the default options, 1,000 callers, 1,000 rounds, 256-byte payloads and one keyed record for each submit.

- **Hosted.** `scripts/m1-compare-refs.sh` builds both commits and runs one warm-up for each, then five alternating pairs of the closed loop, five of the paced variant, and three traced paced repetitions. The workflow `m1-compare` runs it. `tables.py` wrote each `tables.md`.
- **Guest.** The same five alternating pairs, paced and closed loop, through `gated.sh`.
- **Paced.** `SEGMENT_LOG_BENCH_INTERVAL_US=20000`: each caller submits once in 20 ms, and latency runs from the scheduled send ([ADR 0045](../../../decisions/0045-latency-target-is-measured-at-a-paced-rate.md)).
- **Trace.** `SEGMENT_LOG_PIPELINE_OUT` with 200 rounds. `scripts/m1-group-phases.py` gives the wait from a request's admission until its group stops gathering.

Every run validated all its records before and after reopen, with no refusal and no commit or correctness error.

## Paced at 50,000 records/s

Median and range of five runs. The full tables are in [hosted-1/tables.md](hosted-1/tables.md), [hosted-2/tables.md](hosted-2/tables.md), [hosted-3/tables.md](hosted-3/tables.md) and [docker/paced/summary.md](docker/paced/summary.md).

| Comparison | | Mean ms | p50 ms | p99 ms | p999 ms | Groups |
| --- | --- | --- | --- | --- | --- | --- |
| Hosted 1 | before | 1.591 (1.575–1.730) | 1.521 (1.518–1.648) | 4.512 (4.090–5.128) | 7.620 (6.819–8.448) | 13,069 |
| | after | 1.239 (1.234–1.242) | 1.172 (1.171–1.175) | 3.645 (3.578–3.826) | 8.564 (7.158–8.958) | 18,553 |
| Hosted 2 | before | 1.671 (1.645–1.751) | 1.620 (1.607–1.717) | 3.512 (3.425–3.784) | 8.535 (8.268–9.179) | 12,614 |
| | after | 1.227 (1.207–1.253) | 1.183 (1.160–1.194) | 3.309 (3.054–4.137) | 9.655 (8.395–10.308) | 18,718 |
| Hosted 3 | before | 1.750 (1.706–1.764) | 1.646 (1.618–1.669) | 5.027 (4.760–5.287) | 8.624 (8.472–9.070) | 12,350 |
| | after | 1.262 (1.251–1.305) | 1.178 (1.172–1.198) | 4.123 (3.979–4.895) | 8.770 (8.308–9.197) | 18,447 |
| Guest | before | 1.300 (1.235–2.766) | 1.275 (1.167–1.331) | 2.397 (1.976–28.023) | 7.109 (2.405–67.684) | 13,953 |
| | after | 1.061 (0.964–2.006) | 1.020 (0.968–1.081) | 2.406 (1.516–39.117) | 12.653 (1.711–52.662) | 18,885 |

- The p50 is lower after in every pair of every comparison, and the mean in 19 pairs of 20, with separated ranges on the hosted runner. The one exception is a guest pair whose second run met a disk stall.
- The p99 ranges are separated in hosted comparison 1 and overlap in the others.
- The p999 shows no established change. It is set by device flushes of 5 ms and more, which occur on both sides.

### The guest, run by run

| Run | Mean ms | p50 ms | p99 ms | p999 ms | Bound ms | p99 minus bound ms | Groups | Group flush p99 ms | Longest group flush ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| before 1 | 1.281 | 1.167 | 1.976 | 39.546 | 2.169 | −0.193 | 14,636 | 0.415 | 28.624 |
| before 2 | 1.310 | 1.275 | 3.328 | 6.563 | 1.969 | +1.359 | 13,914 | 0.540 | 4.884 |
| before 3 | 1.300 | 1.277 | 2.397 | 7.109 | 2.375 | +0.022 | 13,953 | 0.654 | 5.690 |
| before 4 | 2.766 | 1.331 | 28.023 | 67.684 | 1.941 | +26.082 | 13,037 | 4.234 | 28.621 |
| before 5 | 1.235 | 1.235 | 2.008 | 2.405 | 1.874 | +0.134 | 14,294 | 0.487 | 1.514 |
| after 1 | 1.061 | 1.043 | 1.733 | 7.233 | 2.004 | −0.271 | 18,938 | 0.591 | 7.538 |
| after 2 | 2.006 | 1.081 | 39.117 | 52.662 | 1.726 | +37.391 | 18,358 | 0.992 | 23.119 |
| after 3 | 1.246 | 1.020 | 7.882 | 27.634 | 2.075 | +5.807 | 18,535 | 1.337 | 15.769 |
| after 4 | 1.047 | 0.992 | 2.406 | 12.653 | 2.300 | +0.106 | 18,885 | 0.609 | 10.880 |
| after 5 | 0.964 | 0.968 | 1.516 | 1.711 | 1.913 | −0.396 | 19,017 | 0.342 | 2.903 |

- The guest's virtual disk stalls in some runs. "Before 4", "after 2" and "after 3" each have group flushes of 15 ms and more, and their p99 follows.
- "Before 5" and "after 5" have no group flush above 3 ms. Their p99 is 2.008 ms against a bound of 1.874 ms before, and 1.516 ms against 1.913 ms after. The miss of 0.134 ms matches the 0.13 to 0.25 ms that issue 20 recorded.
- The p99 is within the bound in 1 run of 5 before and 2 of 5 after. The capture of the final tree in the [completion record](../../m1/completion.md), on the same guest in the same hour, has 4 of 5.

## Where the wait goes

Traced paced repetitions on the hosted runner. The wait runs from a request's admission until its group stops gathering.

| Comparison | | Mean ms | p50 ms | p90 ms | p99 ms |
| --- | --- | --- | --- | --- | --- |
| Hosted 1 | before | 0.802–0.835 | 0.761–0.776 | 1.382–1.422 | 2.540–2.999 |
| | after | 0.540–0.572 | 0.509–0.516 | 0.957–0.971 | 1.426–2.349 |
| Hosted 2 | before | 0.777–0.787 | 0.753–0.764 | 1.367–1.401 | 1.604–1.622 |
| | after | 0.533–0.547 | 0.503–0.504 | 0.951–0.954 | 1.086–1.266 |
| Hosted 3 | before | 0.820–0.841 | 0.762–0.789 | 1.386–1.457 | 2.782–3.429 |
| | after | 0.577–0.584 | 0.516–0.517 | 0.973–0.976 | 2.660–2.757 |

Each cell is the range of three repetitions. Before the change, nine requests in ten waited up to 1.4 ms: the rest of the previous flush, then the linger. After it they wait up to 0.97 ms, which is the linger. The p99 above that is the wait behind a slow flush.

## Closed loop

| Comparison | | Records/s | p50 ms | p99 ms | Groups |
| --- | --- | --- | --- | --- | --- |
| Hosted 1 | before | 205,761 (199,330–214,747) | 4.730 (4.513–4.892) | 8.048 (7.674–8.629) | 2,535 |
| | after | 216,273 (210,127–224,267) | 4.454 (4.297–4.592) | 7.934 (7.677–8.177) | 2,023 |
| Hosted 2 | before | 211,575 (198,303–217,668) | 4.644 (4.498–4.965) | 7.266 (6.883–7.646) | 2,574 |
| | after | 214,592 (202,216–222,893) | 4.513 (4.364–4.834) | 7.516 (7.167–7.909) | 2,021 |
| Hosted 3 | before | 216,268 (204,803–219,487) | 4.461 (4.417–4.731) | 7.709 (7.602–9.763) | 2,434 |
| | after | 217,128 (212,068–221,355) | 4.456 (4.358–4.519) | 8.077 (7.367–8.769) | 2,014 |
| Guest | before | 434,600 (424,307–439,852) | 2.047 (2.038–2.083) | 3.972 (3.931–4.053) | 1,204 |
| | after | 412,835 (391,535–414,087) | 2.411 (2.387–2.509) | 3.500 (3.344–4.429) | 1,617 |

- **Hosted runner.** The ranges overlap in all three comparisons. No change is established.
- **Guest.** Throughput is 5% lower and the p50 is 0.36 ms higher, both with separated ranges. The p99 is lower in four pairs of five. The groups are smaller: 618 records for each in place of 831.
- In a closed loop the mean latency is the caller count divided by the throughput, so it follows the throughput. It is not the target comparison ([ADR 0045](../../../decisions/0045-latency-target-is-measured-at-a-paced-rate.md)).

## The first version, and why the group has a limit

The first version of this change had no limit on what a group takes once its linger has ended. It took queued submits for as long as any were there. [first-version/](first-version/) holds three hosted comparisons of it.

| Runner | Records/s before | Records for each group | p99 ms | Records/s after | Records for each group | p99 ms |
| --- | --- | --- | --- | --- | --- | --- |
| AMD EPYC 7763 | 225,288 (217,465–228,628) | 418 | 6.696 (6.470–7.075) | 221,217 (216,601–221,878) | 749 | 7.791 (7.740–7.934) |
| Intel Xeon Platinum 8573C | 200,981 (197,261–209,838) | 394 | 7.261 (6.513–7.409) | 193,711 (184,841–201,102) | 780 | 8.324 (7.849–9.891) |
| Intel Xeon 6973P-C | 219,605 (196,013–234,519) | 448 | 6.508 (6.244–25.535) | 189,093 (171,407–218,642) | 829 | 8.097 (7.078–26.397) |

In the closed loop arrivals never stop, so one group kept gathering. Groups nearly doubled, throughput fell by 2 to 14%, and the p99 rose by 1.1 to 1.6 ms. The paced runs of that version looked like the final ones, so the paced runs alone did not show the fault.

The final version notes the time when the writer first sees the linger over, and a group takes only the submits admitted before that time. The closed-loop tables above are for the final version.

## The same code twice

[same-code/](same-code/) compares two commits with the same engine code on a hosted runner. The second binary of each pair had a p50 0.10 to 0.14 ms lower in the paced runs. That is the size of an order effect on this kind of runner. The paced p50 changes in this record are 0.35 to 0.47 ms on the hosted runner.

## Scenarios

Three scenarios in `segment-log-core/src/acceptance/durability.rs`, each written before the code that it checks.

- `linger_runs_from_the_admission_of_a_groups_first_submit` failed on the earlier writer with `the late group waited another linger after 1.001113292s` ([red-linger.log](red-linger.log)).
- `a_group_takes_only_submits_admitted_before_its_gathering_ended` failed on the first version with three flushes where four are expected: the late submit had joined the group.
- `a_group_formed_after_its_linger_ended_fails_and_recovers_whole` fails a late group's flush and replays each of its storage boundaries through every crash mode. It passes before and after.

A variant that stops at the end of the linger without taking the queued submits fails the first and the third: four flushes where two are expected ([variant-stop-at-the-end.log](variant-stop-at-the-end.log)).

## Limits

- Virtual machines only: a 4-processor hosted runner whose mount does not force a device flush, and a 10-processor guest on a Mac. No local NVMe.
- One offered rate and one caller count. The paced variant has one outstanding request for each caller, so it is a paced closed loop and not a true open loop.
- The guest comparison is five pairs in one quiet window, with disk stalls in three runs of ten.
- The 5% closed-loop cost is from the guest alone. The hosted runner does not show it.
- No run measures the effect of the higher flush rate on other tenants of shared media ([V11](../../../architecture/volume-requirements.md#v11-flush-latency-allows-the-throughput-target)).

## Checks

On the Mac host and in the Linux container, for the final version: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the script tests, and the long run with 100 trace seeds from 10000 (2,700 scenarios) and 30 process-kill repetitions. [checks/](checks/) holds the Mac results.

One long run in the Linux container failed at seed 10030. Its cause was in the test harness's reader and exists on `main` without this change. [Its record](../reader-crash-cancel/README.md) has the saved pair, the replay and the correction. With that correction the long run passes in the container.

## Reproduce

Hosted: put the `m1-compare` label on the pull request, then `gh run download RUN_ID`.

In the pinned image, with both test binaries built by `cargo test -p segment-log-core --lib --release --no-run`:

```sh
SEGMENT_LOG_EVIDENCE_DIR=/data/bench SEGMENT_LOG_BENCH_INTERVAL_US=20000 \
  scripts/m1-compare-latency.sh BEFORE_TEST_BINARY AFTER_TEST_BINARY paced
SEGMENT_LOG_EVIDENCE_DIR=/data/bench scripts/m1-compare-latency.sh BEFORE_TEST_BINARY AFTER_TEST_BINARY latency
python3 docs/evidence/issue20/linger-admission/tables.py DIRECTORY
```
