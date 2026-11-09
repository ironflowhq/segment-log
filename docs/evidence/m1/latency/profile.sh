#!/bin/bash
set -euo pipefail
cd /root/latency/src
source /root/.cargo/env
out=/root/latency/profiles
mkdir -p "$out"
export SEGMENT_LOG_EVIDENCE_DIR=/root/latency/data
export SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_BENCH_ROUNDS=1000 SEGMENT_LOG_DEVICE_ROUNDS=1000
export CARGO_TARGET_DIR=/root/latency/profile-build CARGO_PROFILE_RELEASE_DEBUG=1
cargo test -p segment-log-core --lib --release --no-run --message-format=json > "$out/build.jsonl"
binary="$(python3 -c 'import json; print(next(x["executable"] for x in map(json.loads,open("/root/latency/profiles/build.jsonl")) if x.get("executable")))')"
sha256sum "$binary" > "$out/binary.sha256"
perf version > "$out/perf-version.txt"
perf record -e cpu-clock -F 99 --call-graph dwarf -o "$out/perf.data" -- "$binary" --exact acceptance::evidence::measured_appends_survive_reopen --nocapture > "$out/workload.log" 2> "$out/perf.log"
perf report -i "$out/perf.data" --stdio --no-children --sort comm,symbol --percent-limit 0.5 > "$out/report.txt" 2> "$out/report-errors.txt"

perf report -i "$out/perf.data" --stdio --no-children --comms segment-log-wri --sort symbol -g none --percent-limit 1 > "$out/writer.txt" 2> "$out/writer-errors.txt"
