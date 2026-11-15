#!/bin/bash
# The M6 socket diagnostic (docs/prd.md, M6 row; ADR 0056): append throughput and latency over the
# Unix socket beside the same workload in process. Diagnostics under ADR 0029, not a gate.
# usage: scripts/m6-socket-report.sh OUTPUT_DIRECTORY
#
# Both sides run segment-log-server/examples/socket_bench.rs, so the workload code is the same and
# only the transport differs. The defaults are the PRD append conditions at the api.md defaults:
# 1,000 callers, 256-byte payloads, 1 ms linger, and a paced run at 50,000 records/s for latency
# (ADR 0045). Sides alternate, in process then socket, for each pair, after one warm-up of each.
# Every run starts on a fresh log, because a repeated key is a deduplicated retry with no sync.
# M6_SOCKET_SMALL=1 shrinks the sizes to check the tooling; its numbers are not evidence.
set -euo pipefail
out="${1:?usage: scripts/m6-socket-report.sh OUTPUT_DIRECTORY}"
if [ -n "${M6_SOCKET_SMALL:-}" ]; then
    : "${M6_CALLERS:=20}" "${M6_ROUNDS:=50}" "${M6_PAIRS:=2}"
fi
callers="${M6_CALLERS:-1000}"
rounds="${M6_ROUNDS:-1000}"
pairs="${M6_PAIRS:-5}"
# 1,000 callers each sending once per 20 ms offer 50,000 records/s, the throughput target.
interval_us="${M6_INTERVAL_US:-20000}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
root="$(git rev-parse --show-toplevel)"
work="$(mktemp -d)"
data="${SEGMENT_LOG_EVIDENCE_DIR:-$work/data}/m6-$$"
server_pid=""
cleanup() {
    status=$?
    printf "exit=%s\n" "$status" > "$out/exit-status.txt"
    [ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null
    rm -rf "$work" "$data"
}
trap cleanup EXIT
mkdir -p "$data"

cargo build --release --locked -p segment-log-server --bin segment-log-server --example socket_bench > "$out/build.log" 2>&1
server="$root/target/release/segment-log-server"
bench="$root/target/release/examples/socket_bench"
sha256sum "$server" "$bench" 2>/dev/null > "$out/binaries.sha256" || shasum -a 256 "$server" "$bench" > "$out/binaries.sha256"

{
    date -u
    echo "head=$(git -C "$root" rev-parse HEAD)"
    echo "callers=$callers rounds=$rounds pairs=$pairs interval_us=$interval_us small=${M6_SOCKET_SMALL:-0}"
    rustc -Vv
    uname -a
    if [ "$(uname -s)" = Linux ]; then
        cat /etc/os-release
        lscpu
        free -b
        findmnt -T "$data"
        lsblk -o NAME,TYPE,SIZE,ROTA,MODEL,MOUNTPOINTS 2>&1 || true
    else
        sw_vers 2>&1 || true
        sysctl -n machdep.cpu.brand_string hw.memsize 2>&1 || true
    fi
    echo "cpus=$(getconf _NPROCESSORS_ONLN)"
    df -k "$data"
    uptime
} > "$out/environment.txt"

# run SIDE MODE LOG: one bench run on a fresh log directory.
run() {
    local side="$1" mode="$2" log="$3" dir
    dir="$(mktemp -d "$data/run-XXXX")"
    # A number or nothing, left unquoted so that a closed run passes no argument.
    local interval=""
    [ "$mode" = paced ] && interval="$interval_us"
    uptime > "${log%.log}.load.txt"
    if [ "$side" = inproc ]; then
        "$bench" --inproc "$dir/log" "$callers" "$rounds" $interval > "$log"
    else
        "$server" --dir "$dir/log" --unix "$dir/socket" --max-connections "$callers" 2> "${log%.log}.server.txt" &
        server_pid=$!
        for _ in $(seq 100); do [ -S "$dir/socket" ] && break; sleep 0.1; done
        "$bench" --unix "$dir/socket" "$callers" "$rounds" $interval > "$log"
        kill -TERM "$server_pid"
        wait "$server_pid"
        server_pid=""
    fi
    rm -rf "$dir"
}

for mode in closed paced; do
    mkdir -p "$out/$mode"
    for side in inproc unix; do
        run "$side" "$mode" "$out/$mode/warmup-$side.log"
    done
    for pair in $(seq "$pairs"); do
        for side in inproc unix; do
            run "$side" "$mode" "$out/$mode/$side-$pair.log"
        done
    done
done
python3 "$root/scripts/m6-socket-summary.py" "$out" > "$out/summary.md"
cat "$out/summary.md"
