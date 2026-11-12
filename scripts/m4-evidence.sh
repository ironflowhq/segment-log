#!/bin/bash
# Captures the M4 gate measurements (docs/prd.md, M4 row): open time cold and warm with every
# record keyed, key-cache memory, and keyed append throughput and p99 latency beside the M3 code.
# A closed loop of unkeyed appends on the same keyed history measures what the key lookup costs on
# each side (ADR 0051).
# usage: scripts/m4-evidence.sh OUTPUT_DIRECTORY [HEAD [BASE]]
#
# HEAD defaults to the checked-out commit, BASE to the M3 code (main at 0ffc772). Each side is
# built in its own worktree from this checkout's segment-log-core/examples/sealed_probe.rs, which
# uses only the public API both sides share, so both run the same workload code.
#
# The defaults are the PRD open-time conditions at the api.md defaults: 100 sealed segments of the
# 64 MiB default roll and a 4 MiB active segment, and the PRD append conditions: 1,000 callers,
# 256-byte payloads, 1 ms linger, paced at 50,000 records/s for the latency run (ADR 0045).
# M4_EVIDENCE_SMALL=1 shrinks every size to check the tooling; its numbers are not evidence.
# Cold opens drop the page cache, which needs root; M4_COLD=off, or no root, skips them.
set -euo pipefail
out="${1:?usage: scripts/m4-evidence.sh OUTPUT_DIRECTORY [HEAD [BASE]]}"
head="$(git rev-parse --verify "${2:-HEAD}^{commit}")"
base="$(git rev-parse --verify "${3:-0ffc77259ade423091eea503b282f28bf674745c}^{commit}")"
if [ -n "${M4_EVIDENCE_SMALL:-}" ]; then
    : "${M4_SEALED:=3}" "${M4_ROLL_BYTES:=1048576}" "${M4_ACTIVE_BYTES:=262144}"
    : "${M4_OPEN_RUNS:=3}" "${M4_CALLERS:=20}" "${M4_ROUNDS:=50}" "${M4_PAIRS:=2}"
fi
sealed="${M4_SEALED:-100}"
roll_bytes="${M4_ROLL_BYTES:-67108864}"
active_bytes="${M4_ACTIVE_BYTES:-4194304}"
open_runs="${M4_OPEN_RUNS:-10}"
callers="${M4_CALLERS:-1000}"
rounds="${M4_ROUNDS:-1000}"
pairs="${M4_PAIRS:-5}"
# 1,000 callers each sending once per 20 ms offer 50,000 records/s, the throughput target.
interval_us="${M4_INTERVAL_US:-20000}"
# Each timed append run waits this long after open, so that the log key filter's background build
# (ADR 0051) is outside it. 0 times the build too.
export SEALED_PROBE_SETTLE_MS="${M4_SETTLE_MS:-0}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
root="$(git rev-parse --show-toplevel)"
scripts="$root/scripts"
probe_src="$root/segment-log-core/examples/sealed_probe.rs"
work="$(mktemp -d)"
data="${SEGMENT_LOG_EVIDENCE_DIR:-$work/data}/m4-$$"
cleanup() {
    status=$?
    printf "exit=%s\n" "$status" > "$out/exit-status.txt"
    for side in before after; do
        git -C "$root" worktree remove --force "$work/$side" 2>/dev/null || true
    done
    rm -rf "$work" "$data"
}
trap cleanup EXIT
mkdir -p "$data"

cold=no
cold_reason=""
case "${M4_COLD:-auto}:$(uname -s)" in
    off:*) cold_reason="M4_COLD=off" ;;
    *:Linux) if (echo 3 > /proc/sys/vm/drop_caches) 2>/dev/null || sudo -n sh -c 'echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null; then cold=yes; else cold_reason="cannot write /proc/sys/vm/drop_caches"; fi ;;
    *:Darwin) if command -v purge > /dev/null && sudo -n true 2>/dev/null; then cold=yes; else cold_reason="no root to run purge"; fi ;;
    *) cold_reason="no page-cache drop on $(uname -s)" ;;
esac
sha() {
    if command -v sha256sum > /dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}
timer=()
if /usr/bin/time -v true > /dev/null 2>&1; then
    timer=(/usr/bin/time -v)
fi

{
    date -u
    echo "head=$head"
    echo "base=$base"
    echo "checkout=$(git -C "$root" rev-parse HEAD)"
    sha "$probe_src" | sed "s|$root/||"
    echo "sealed=$sealed roll_bytes=$roll_bytes active_bytes=$active_bytes open_runs=$open_runs callers=$callers rounds=$rounds pairs=$pairs interval_us=$interval_us settle_ms=$SEALED_PROBE_SETTLE_MS small=${M4_EVIDENCE_SMALL:-0}"
    echo "cold_opens=$cold${cold_reason:+ ($cold_reason)}"
    rustc -Vv
    cargo -V
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
        mount | grep -F " on $(df "$data" | awk 'NR==2 {print $NF}') " || true
    fi
    echo "cpus=$(getconf _NPROCESSORS_ONLN)"
    df -k "$data"
    uptime
} > "$out/environment.txt"

build() { # build SIDE COMMIT: prints the release probe binary
    git -C "$root" worktree add --quiet --detach "$work/$1" "$2"
    cp "$probe_src" "$work/$1/segment-log-core/examples/sealed_probe.rs"
    (cd "$work/$1" && CARGO_TARGET_DIR="$work/target-$1" \
        cargo build --release -p segment-log-core --example sealed_probe) > "$out/build-$1.log" 2>&1
    echo "$work/target-$1/release/examples/sealed_probe"
}
before="$(build before "$base")"
after="$(build after "$head")"
sha "$before" "$after" | sed "s|$work/||" > "$out/binaries.sha256"
probe() { # probe SIDE ARGS...
    local binary="$after"
    [ "$1" = before ] && binary="$before"
    shift
    "$binary" "$@"
}

# Each side builds its own keyed fixture, because the M3 code cannot read M4 closing frames.
for fixture in after-keyed after-unkeyed before-keyed; do
    probe "${fixture%%-*}" build "$data/$fixture" "${fixture#*-}" "$sealed" "$roll_bytes" "$active_bytes" \
        > "$out/fixture-$fixture.log" 2>&1
done

mkdir -p "$out/open"
for fixture in keyed unkeyed; do
    if [ "$cold" = yes ]; then
        probe after open "$data/after-$fixture" cold "$open_runs" > "$out/open/$fixture-cold.log" 2>&1
    fi
    probe after open "$data/after-$fixture" warm "$open_runs" > "$out/open/$fixture-warm.log" 2>&1
done
[ -n "$cold_reason" ] && echo "cold opens not run: $cold_reason" > "$out/open/cold-not-applicable.txt"

files() { # files DIR: prints each file with its size and modification time
    python3 -c 'import os,sys
for d, _, names in sorted(os.walk(sys.argv[1])):
    for n in sorted(names):
        s = os.stat(os.path.join(d, n))
        print(os.path.join(d, n), s.st_size, s.st_mtime_ns)' "$1"
}
# A run shares the fixture's sealed segments through hard links and copies the rest, so that
# every run starts from the same history without rewriting it. Sealed segments never change (I7);
# the check after the runs fails if a run wrote to one.
prepare() { # prepare FIXTURE RUN
    mkdir -p "$2/log"
    cp "$1/manifest" "$2/manifest"
    local segments=("$1"/log/*.seg) path
    for path in "$1"/log/*; do
        if [[ "$path" == *.seg && "$path" != "${segments[${#segments[@]}-1]}" ]]; then
            ln "$path" "$2/log/${path##*/}"
        else
            cp -p "$path" "$2/log/${path##*/}"
        fi
    done
}
quiet() { # quiet FILE: records the load, and on Linux waits for an idle processor first
    uptime >> "$1"
    if [ -r /proc/stat ]; then
        python3 "$scripts/m1-quiet.py" "$1"
    fi
}
files "$data/before-keyed" > "$work/before-keyed.files"
files "$data/after-keyed" > "$work/after-keyed.files"
for side in before after; do
    prepare "$data/$side-keyed" "$data/run"
    probe "$side" append "$data/run" keyed "$callers" 20 > "$out/warmup-$side.log" 2>&1
    rm -rf "$data/run"
done
# closed-unkeyed sends unkeyed records on the same keyed fixture, so that against closed it
# isolates the lookup on each side.
for mode in closed closed-unkeyed paced; do
    mkdir -p "$out/append/$mode"
    keys=keyed
    [ "$mode" = closed-unkeyed ] && keys=unkeyed
    extra=""
    [ "$mode" = paced ] && extra="$interval_us"
    for pair in $(seq 1 "$pairs"); do
        for side in before after; do
            prepare "$data/$side-keyed" "$data/run"
            quiet "$out/append/$mode/load-$side-$pair.txt"
            binary="$after"
            [ "$side" = before ] && binary="$before"
            # shellcheck disable=SC2086 # extra is empty for the closed loop
            ${timer[@]+"${timer[@]}"} "$binary" append "$data/run" "$keys" "$callers" "$rounds" $extra \
                > "$out/append/$mode/$side-$pair.log" 2>&1
            rm -rf "$data/run"
        done
    done
done
for side in before after; do
    files "$data/$side-keyed" | diff "$work/$side-keyed.files" - > "$out/fixture-$side-keyed-changes.txt" ||
        { echo "a run changed the $side fixture" >&2; exit 1; }
done
python3 "$scripts/m4-summary.py" "$out" > "$out/summary.md"
cat "$out/summary.md"
