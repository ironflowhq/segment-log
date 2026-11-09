#!/bin/bash
set -euo pipefail
cd /root/latency/src
out=/root/latency/out
apt-get update > "$out/apt.log" 2>&1
apt-get install -y build-essential curl python3 linux-tools-generic strace time >> "$out/apt.log" 2>&1
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.99.1 > "$out/rustup.log" 2>&1
source /root/.cargo/env
{ date -u; rustc -Vv; cargo -V; uname -a; lscpu; free -b; lsblk -o NAME,SIZE,TYPE,MODEL; findmnt -T /root/latency/data; uptime; ps -eo pid,comm,pcpu --sort=-pcpu | head -20; } > "$out/environment.txt"
cargo test -p segment-log-core --lib --release --no-run --message-format=json > "$out/build.jsonl"
binary="$(python3 -c 'import json; print(next(x["executable"] for x in map(json.loads,open("/root/latency/out/build.jsonl")) if x.get("executable")))')"
cp "$binary" /root/latency/baseline-bin
export SEGMENT_LOG_EVIDENCE_DIR=/root/latency/data
export SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_DEVICE_ROUNDS=1000
SEGMENT_LOG_BENCH_ROUNDS=20 "$binary" --exact acceptance::evidence::measured_appends_survive_reopen --nocapture > "$out/warmup.log" 2>&1
export SEGMENT_LOG_BENCH_ROUNDS=1000
for run in 1 2 3 4 5; do
 uptime > "$out/load-$run.txt"
 SEGMENT_LOG_GROUPS_OUT="$out/groups-$run.tsv" /usr/bin/time -v "$binary" --exact acceptance::evidence::measured_appends_survive_reopen --nocapture > "$out/run-$run.log" 2>&1
done
printf 'PASS\n' > "$out/baseline-status.txt"
