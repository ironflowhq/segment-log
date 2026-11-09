#!/bin/bash
set -euo pipefail
trap 'printf "exit=%s\n" "$?" > /out/exit-status.txt' EXIT
export CARGO_TARGET_DIR=/build
export SEGMENT_LOG_EVIDENCE_DIR=/data
mkdir -p /data
apt-get update
apt-get install -y --no-install-recommends strace sysstat time python3
{
    date -u
    rustc -Vv
    cargo -V
    strace -V
    uname -a
    cat /etc/os-release
    lscpu
    free -b
    df -T /data /src /out
    findmnt -T /data
    cat /proc/mounts
    cat /sys/fs/cgroup/pids.max
    cat /sys/fs/cgroup/memory.max
    cat /sys/fs/cgroup/cpu.max
    uptime
    nproc
    ps -eo pid,comm,pcpu,pmem --sort=-pcpu | head -20
} > /out/environment.txt
cargo test -p segment-log-core --lib --release --no-run --message-format=json > /out/build.jsonl
binary="$(python3 -c 'import json; print(next(x["executable"] for x in map(json.loads,open("/out/build.jsonl")) if x.get("executable")))')"
sync_test=acceptance::evidence::real_storage_sync_phases
bench_test=acceptance::evidence::measured_appends_survive_reopen
if [ "${M1_PROFILE_ONLY:-0}" = 1 ]; then
    SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_BENCH_ROUNDS=1000 SEGMENT_LOG_DEVICE_ROUNDS=1000         python3 scripts/m1-profile.py "$binary" "$bench_test" /out
    exit
fi
cargo fmt --all --check > /out/fmt.log 2>&1
cargo clippy --workspace --all-targets -- -D warnings > /out/clippy.log 2>&1
cargo test --workspace > /out/tests.log 2>&1
cargo test -p segment-log-core cited_cases_are_defined_and_uncited_cases_are_reported -- --nocapture > /out/coverage.log 2>&1
RUSTFLAGS='--cfg fuzzing' cargo check -p segment-log-core > /out/fuzz.log 2>&1
strace -f -tt -T -yy -s 256 -e trace=fdatasync,fsync,write -o /out/syscalls.trace \
    "$binary" --exact "$sync_test" --nocapture > /out/sync-phases.log 2>&1
python3 scripts/m1-verify-syscalls.py /out/syscalls.trace > /out/syscalls-verified.txt
export SEGMENT_LOG_BENCH_CALLERS=1000
export SEGMENT_LOG_DEVICE_ROUNDS=1000
SEGMENT_LOG_BENCH_ROUNDS=20 "$binary" --exact "$bench_test" --nocapture > /out/warmup.log 2>&1
export SEGMENT_LOG_BENCH_ROUNDS=1000
for run in 1 2 3 4 5; do
    uptime > "/out/load-$run.txt"
    SEGMENT_LOG_GROUPS_OUT="/out/groups-$run.tsv" /usr/bin/time -v \
        "$binary" --exact "$bench_test" --nocapture > "/out/run-$run.log" 2>&1
done
# Separate diagnostic run. These times are excluded from benchmark statistics.
SEGMENT_LOG_BENCH_ROUNDS=50 strace -f -c -o /out/diagnostic-strace.txt \
    "$binary" --exact "$bench_test" --nocapture > /out/diagnostic.log 2>&1
SEGMENT_LOG_BENCH_ROUNDS=1000 python3 scripts/m1-profile.py "$binary" "$bench_test" /out
