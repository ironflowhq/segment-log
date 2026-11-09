#!/bin/bash
set -euo pipefail
out="${1:?usage: scripts/m1-latency-linux.sh OUTPUT_DIRECTORY}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
export SEGMENT_LOG_EVIDENCE_DIR="${SEGMENT_LOG_EVIDENCE_DIR:-$out/data}"
export SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_BENCH_ROUNDS=1000 SEGMENT_LOG_DEVICE_ROUNDS=1000
unset SEGMENT_LOG_BENCH_CPU SEGMENT_LOG_GROUPS_OUT
{
    date -u
    git rev-parse HEAD
    git diff --stat
    rustc -Vv
    uname -a
    lscpu
    free -b
    mkdir -p "$SEGMENT_LOG_EVIDENCE_DIR"
    findmnt -T "$SEGMENT_LOG_EVIDENCE_DIR"
    cat /proc/sys/kernel/sched_schedstats
    uptime
} > "$out/environment.txt"
cargo test -p segment-log-core --lib --release --no-run --message-format=json > "$out/build.jsonl"
binary="$(python3 -c 'import json,sys; print(next(x["executable"] for x in map(json.loads,open(sys.argv[1])) if x.get("executable")))' "$out/build.jsonl")"
sha256sum "$binary" > "$out/binary.sha256"
test=acceptance::evidence::measured_appends_survive_reopen
for mode in measured real; do
    SEGMENT_LOG_BENCH_STORAGE="$mode" SEGMENT_LOG_BENCH_ROUNDS=20 "$binary" --exact "$test" --nocapture > "$out/warmup-$mode.log" 2>&1
done
for run in 1 2 3 4 5; do
    for mode in measured real; do
        uptime > "$out/load-$mode-$run.txt"
        SEGMENT_LOG_BENCH_STORAGE="$mode" /usr/bin/time -v "$binary" --exact "$test" --nocapture > "$out/$mode-$run.log" 2>&1
    done
done
SEGMENT_LOG_BENCH_STORAGE=measured SEGMENT_LOG_BENCH_CPU=1 "$binary" --exact "$test" --nocapture > "$out/cpu.log" 2>&1
