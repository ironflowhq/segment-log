#!/bin/bash
# Compares the append benchmark of two commits on this Linux host:
# usage scripts/m1-compare-refs.sh BASE HEAD OUTPUT_DIRECTORY.
# It builds each commit in its own worktree, then runs five alternating pairs of the default
# closed loop and five of the paced variant at the throughput target's rate (ADR 0045).
set -euo pipefail
base="$(git rev-parse --verify "${1:?usage: scripts/m1-compare-refs.sh BASE HEAD OUTPUT_DIRECTORY}^{commit}")"
head="$(git rev-parse --verify "${2:?missing head}^{commit}")"
out="${3:?missing output directory}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
scripts="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'git worktree remove --force "$work/before" 2>/dev/null; git worktree remove --force "$work/after" 2>/dev/null; rm -rf "$work"' EXIT
export SEGMENT_LOG_EVIDENCE_DIR="${SEGMENT_LOG_EVIDENCE_DIR:-$work/data}"
mkdir -p "$SEGMENT_LOG_EVIDENCE_DIR"
{
    date -u
    echo "before=$base"
    echo "after=$head"
    rustc -Vv
    uname -a
    lscpu
    free -b
    df -T "$SEGMENT_LOG_EVIDENCE_DIR"
    findmnt -T "$SEGMENT_LOG_EVIDENCE_DIR"
    lsblk -o NAME,TYPE,SIZE,ROTA,MODEL,MOUNTPOINTS 2>&1 || true
    uptime
    nproc
} > "$out/environment.txt"
build() { # build SIDE COMMIT : prints the path of the release test binary
    git worktree add --quiet --detach "$work/$1" "$2"
    (cd "$work/$1" && CARGO_TARGET_DIR="$work/target-$1" \
        cargo test -p segment-log-core --lib --release --no-run --message-format=json > "$out/build-$1.jsonl")
    python3 -c 'import json,sys; print(next(x["executable"] for x in map(json.loads,open(sys.argv[1])) if x.get("executable")))' "$out/build-$1.jsonl"
}
before="$(build before "$base")"
after="$(build after "$head")"
"$scripts/m1-compare-latency.sh" "$before" "$after" "$out/latency"
python3 "$scripts/m1-latency-summary.py" "$out/latency" before after > "$out/latency/summary.md"
SEGMENT_LOG_BENCH_INTERVAL_US=20000 "$scripts/m1-compare-latency.sh" "$before" "$after" "$out/paced"
python3 "$scripts/../docs/evidence/issue20/paced-benchmark/paced/summarize.py" "$out/paced" before after > "$out/paced/summary.md"
grep -H "^METRIC group_fdatasync_ns" "$out"/paced/before-?.log "$out"/paced/after-?.log > "$out/paced/group-flushes.txt" || true
# Three alternating traced repetitions of the paced variant split each request's latency into
# phases. Tracing changes the workload, so these runs enter no latency table.
mkdir -p "$out/trace"
test=acceptance::evidence::measured_appends_survive_reopen
for repetition in 1 2 3; do
    for side in before after; do
        binary="$before"
        [ "$side" = after ] && binary="$after"
        SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_BENCH_ROUNDS=200 SEGMENT_LOG_DEVICE_ROUNDS=200 \
            SEGMENT_LOG_BENCH_INTERVAL_US=20000 SEGMENT_LOG_PIPELINE_OUT="$work/trace.tsv" \
            "$binary" --exact "$test" --nocapture > "$out/trace/$side-$repetition.log" 2>&1
        python3 "$scripts/m1-group-phases.py" "$work/trace.tsv" > "$out/trace/$side-$repetition.phases.txt"
        rm -f "$work/trace.tsv" "$work/trace.groups.tsv"
    done
done
cat "$out/latency/summary.md" "$out/paced/summary.md"
grep -H "gather_wait" "$out"/trace/*.phases.txt
