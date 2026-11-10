#!/bin/bash
# Captures the M1 completion evidence on Linux. Run from the repository root.
# The defaults are the container paths that scripts/m1-evidence-linux.sh mounts. A native host,
# such as a hosted runner, sets M1_OUT, CARGO_TARGET_DIR and SEGMENT_LOG_EVIDENCE_DIR instead.
set -euo pipefail
out="${M1_OUT:-/out}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
trap 'printf "exit=%s\n" "$?" > "$out/exit-status.txt"' EXIT
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/build}"
export SEGMENT_LOG_EVIDENCE_DIR="${SEGMENT_LOG_EVIDENCE_DIR:-/data}"
mkdir -p "$SEGMENT_LOG_EVIDENCE_DIR"
sudo=""
[ "$(id -u)" = 0 ] || sudo=sudo
$sudo apt-get update
$sudo apt-get install -y --no-install-recommends strace sysstat time python3
{
    date -u
    git rev-parse HEAD 2>&1 || true
    rustc -Vv
    cargo -V
    strace -V
    uname -a
    cat /etc/os-release
    lscpu
    free -b
    df -T "$SEGMENT_LOG_EVIDENCE_DIR" "$PWD" "$out"
    findmnt -T "$SEGMENT_LOG_EVIDENCE_DIR"
    lsblk -o NAME,TYPE,SIZE,ROTA,MODEL,MOUNTPOINTS 2>&1 || true
    cat /proc/mounts
    # A container has these limits. A native host may not.
    cat /sys/fs/cgroup/pids.max /sys/fs/cgroup/memory.max /sys/fs/cgroup/cpu.max 2>&1 || true
    uptime
    nproc
    ps -eo pid,comm,pcpu,pmem --sort=-pcpu | head -20
} > "$out/environment.txt"
cargo test -p segment-log-core --lib --release --no-run --message-format=json > "$out/build.jsonl"
binary="$(python3 -c 'import json,sys; print(next(x["executable"] for x in map(json.loads,open(sys.argv[1])) if x.get("executable")))' "$out/build.jsonl")"
sha256sum "$binary" > "$out/binary.sha256"
sync_test=acceptance::evidence::real_storage_sync_phases
bench_test=acceptance::evidence::measured_appends_survive_reopen
if [ "${M1_PROFILE_ONLY:-0}" = 1 ]; then
    SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_BENCH_ROUNDS=1000 SEGMENT_LOG_DEVICE_ROUNDS=1000 \
        python3 scripts/m1-profile.py "$binary" "$bench_test" "$out"
    exit
fi
cargo fmt --all --check > "$out/fmt.log" 2>&1
cargo clippy --workspace --all-targets -- -D warnings > "$out/clippy.log" 2>&1
cargo test --workspace > "$out/tests.log" 2>&1
python3 -B -m unittest discover -s scripts -p 'test_m1_*.py' > "$out/verifier-tests.log" 2>&1
cargo test -p segment-log-core cited_cases_are_defined_and_uncited_cases_are_reported -- --nocapture > "$out/coverage.log" 2>&1
RUSTFLAGS='--cfg fuzzing' cargo check -p segment-log-core > "$out/fuzz.log" 2>&1
strace -f -tt -T -yy -s 256 -e trace=fdatasync,fsync,write -o "$out/syscalls.trace" \
    "$binary" --exact "$sync_test" --nocapture > "$out/sync-phases.log" 2>&1
python3 scripts/m1-verify-syscalls.py "$out/syscalls.trace" > "$out/syscalls-verified.txt"
export SEGMENT_LOG_BENCH_CALLERS=1000
export SEGMENT_LOG_DEVICE_ROUNDS=1000
SEGMENT_LOG_BENCH_ROUNDS=20 "$binary" --exact "$bench_test" --nocapture > "$out/warmup.log" 2>&1
export SEGMENT_LOG_BENCH_ROUNDS=1000
for run in 1 2 3 4 5; do
    uptime > "$out/load-$run.txt"
    python3 scripts/m1-quiet.py "$out/load-$run.txt"
    SEGMENT_LOG_GROUPS_OUT="$out/groups-$run.tsv" /usr/bin/time -v \
        "$binary" --exact "$bench_test" --nocapture > "$out/run-$run.log" 2>&1
done
# The paced variant measures the latency target at the throughput target's rate (ADR 0045):
# 1,000 callers that each submit once in 20 ms offer 50,000 records/s.
for run in 1 2 3 4 5; do
    uptime > "$out/paced-load-$run.txt"
    python3 scripts/m1-quiet.py "$out/paced-load-$run.txt"
    SEGMENT_LOG_BENCH_INTERVAL_US=20000 SEGMENT_LOG_GROUPS_OUT="$out/paced-groups-$run.tsv" /usr/bin/time -v \
        "$binary" --exact "$bench_test" --nocapture > "$out/paced-$run.log" 2>&1
done
python3 scripts/m1-summarize.py "$out" > "$out/summary.md"
# Separate diagnostic run. These times are excluded from benchmark statistics.
SEGMENT_LOG_BENCH_ROUNDS=50 strace -f -c -o "$out/diagnostic-strace.txt" \
    "$binary" --exact "$bench_test" --nocapture > "$out/diagnostic.log" 2>&1
SEGMENT_LOG_BENCH_ROUNDS=1000 python3 scripts/m1-profile.py "$binary" "$bench_test" "$out"
