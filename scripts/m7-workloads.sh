#!/usr/bin/env bash
# The larger and longer workloads of issue 47, on the qualified host's volume drive, run by
# m7-session.sh while that host is up anyway. Each case is the M1 benchmark
# (acceptance::evidence::measured_appends_survive_reopen) at a larger size: its APPEND, END, ROLL
# and REOPEN lines, /usr/bin/time -v, and the resident memory sampled every 5 s.
#
#   scripts/m7-workloads.sh OUT_DIR
#
# Cases: callers (4,000 callers, 4M records), history (10M records across many segments, reopen
# time), soak (60M records, about 18 GB, resident memory over time). The resident memory includes
# the harness's own per-record arrays, 16 bytes per record.
set -euo pipefail
out=${1:?usage: m7-workloads.sh OUT_DIR}
mkdir -p "$out"
out=$(cd "$out" && pwd)
export PATH=$HOME/.cargo/bin:$PATH
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/.cache/segment-log-target}
export SEGMENT_LOG_EVIDENCE_DIR=${SEGMENT_LOG_EVIDENCE_DIR:-/tmp/m7-workloads}
mkdir -p "$SEGMENT_LOG_EVIDENCE_DIR"
command -v /usr/bin/time >/dev/null || { echo "needs /usr/bin/time (apt-get install time)" >&2; exit 1; }
findmnt -T "$SEGMENT_LOG_EVIDENCE_DIR" >"$out/environment.txt"
git rev-parse HEAD >>"$out/environment.txt" 2>&1 || echo "source commit: unknown" >>"$out/environment.txt"
cargo test -p segment-log-core --lib --release --no-run --message-format=json >"$out/build.jsonl"
binary=$(python3 -c 'import json,sys; print(next(x["executable"] for x in map(json.loads,open(sys.argv[1])) if x.get("executable")))' "$out/build.jsonl")
test=acceptance::evidence::measured_appends_survive_reopen

# run <name> <callers> <rounds> <deadline s>
run() {
	local name=$1 rc=0
	echo "== $name: $2 callers x $3 rounds"
	SEGMENT_LOG_BENCH_CALLERS=$2 SEGMENT_LOG_BENCH_ROUNDS=$3 SEGMENT_LOG_DEVICE_ROUNDS=100 \
		SEGMENT_LOG_BENCH_DEADLINE_S=$4 /usr/bin/time -v -o "$out/$name.time" \
		"$binary" --exact "$test" --nocapture >"$out/$name.log" 2>&1 &
	local pid=$!
	echo -e "elapsed_s\trss_kib" >"$out/$name.rss.tsv"
	local start=$SECONDS child
	while kill -0 "$pid" 2>/dev/null; do
		child=$(pgrep -P "$pid" | head -n 1)
		[ -z "$child" ] || printf '%s\t%s\n' $((SECONDS - start)) \
			"$(awk '/^VmRSS/ { print $2 }' "/proc/$child/status" 2>/dev/null)" >>"$out/$name.rss.tsv"
		sleep 5
	done
	wait "$pid" || rc=$?
	grep -E '^(APPEND|END|REOPEN)' "$out/$name.log" || true
	grep -E 'Maximum resident' "$out/$name.time" || true
	[ "$rc" = 0 ] || { echo "FAIL: $name exited $rc" >&2; return 1; }
}

rc=0
run callers 4000 1000 600 || rc=1
run history 1000 10000 1200 || rc=1
run soak 1000 60000 3600 || rc=1
[ "$rc" = 0 ] && echo "== workloads passed" || echo "== workloads failed"
exit "$rc"
