#!/bin/bash
set -euo pipefail
before="${1:?usage: scripts/m1-compare-latency.sh BEFORE_BINARY AFTER_BINARY OUTPUT_DIRECTORY}"
after="${2:?missing after binary}"
out="${3:?missing output directory}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
export SEGMENT_LOG_EVIDENCE_DIR="${SEGMENT_LOG_EVIDENCE_DIR:-$out/data}"
export SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_BENCH_ROUNDS=1000 SEGMENT_LOG_DEVICE_ROUNDS=1000
unset SEGMENT_LOG_BENCH_CPU SEGMENT_LOG_GROUPS_OUT SEGMENT_LOG_BENCH_STORAGE
sha256sum "$before" "$after" > "$out/binaries.sha256"
test=acceptance::evidence::measured_appends_survive_reopen
SEGMENT_LOG_BENCH_ROUNDS=20 "$before" --exact "$test" --nocapture > "$out/warmup-before.log" 2>&1
SEGMENT_LOG_BENCH_ROUNDS=20 "$after" --exact "$test" --nocapture > "$out/warmup-after.log" 2>&1
quiet="$(dirname "$0")/m1-quiet.py"
for run in 1 2 3 4 5; do
    uptime > "$out/load-before-$run.txt"
    python3 "$quiet" "$out/load-before-$run.txt"
    /usr/bin/time -v "$before" --exact "$test" --nocapture > "$out/before-$run.log" 2>&1
    uptime > "$out/load-after-$run.txt"
    python3 "$quiet" "$out/load-after-$run.txt"
    /usr/bin/time -v "$after" --exact "$test" --nocapture > "$out/after-$run.log" 2>&1
done
SEGMENT_LOG_BENCH_CPU=1 "$after" --exact "$test" --nocapture > "$out/cpu-after.log" 2>&1
