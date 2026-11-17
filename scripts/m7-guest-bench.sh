#!/usr/bin/env bash
# The M7 guest benchmark on the ADR 0059 stack, run as root on the VMM host after
# scripts/m7/setup.sh: the M6 socket workload (segment-log-server/examples/socket_bench.rs) against
# the server in a Firecracker guest over hybrid vsock, paired with the same workload in process on
# the host's volume filesystem. Closed loop measures throughput; paced at 50,000 records/s measures
# latency (ADR 0045). Each mode runs one warm-up per side, then 5 pairs, each on a fresh log.
#
#   scripts/m7-guest-bench.sh OUT_DIR
#
# M7_CALLERS, M7_ROUNDS, M7_PAIRS and M7_INTERVAL_US change the workload; M7_VCPUS and M7_MEM_MIB
# size the guest (defaults 2 and 1024 here, so 1,000 connections fit).
set -euo pipefail
out=${1:?usage: m7-guest-bench.sh OUT_DIR}
callers=${M7_CALLERS:-1000}
rounds=${M7_ROUNDS:-1000}
pairs=${M7_PAIRS:-5}
interval_us=${M7_INTERVAL_US:-20000}
export M7_VCPUS=${M7_VCPUS:-2} M7_MEM_MIB=${M7_MEM_MIB:-1024} M7_LOG_BYTES=${M7_LOG_BYTES:-4G}
# shellcheck source=m7/setup.sh
source "$(dirname "$0")/m7/setup.sh"
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d "${M7_WORK:-/tmp}/m7-gb-XXXX")
vm=''
cleanup() {
	set +e
	[ -n "$vm" ] && { kill -KILL "$vm"; wait "$vm"; } 2>/dev/null
	rm -rf "$work"
}
trap cleanup EXIT
cargo build -q --release -p segment-log-server --example socket_bench >"$out/build.log" 2>&1
bench=$CARGO_TARGET_DIR/release/examples/socket_bench
{
	date -u
	echo "head=$(git -C "$repo" rev-parse HEAD)"
	echo "callers=$callers rounds=$rounds pairs=$pairs interval_us=$interval_us vcpus=$M7_VCPUS mem_mib=$M7_MEM_MIB"
	uname -a
	lscpu | grep -E "Model name|^CPU\\(s\\)"
	free -b
	findmnt -T "$work"
	"$M7_CACHE/firecracker" --version | head -n 1
	uptime
} >"$out/environment.txt"

run() { # run <side> <mode> <log>
	local side=$1 mode=$2 log=$3 dir interval=()
	dir=$(mktemp -d "$work/run-XXXX")
	[ "$mode" = paced ] && interval=("$interval_us")
	uptime >"${log%.log}.load.txt"
	if [ "$side" = inproc ]; then
		"$bench" --inproc "$dir/log" "$callers" "$rounds" "${interval[@]}" >"$log"
	else
		"$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 --max-connections "$callers" \
			</dev/null 2>"${log%.log}.vmm.txt" &
		vm=$!
		"$M7_CACHE/powercut" ready -address "$dir/v.sock:5000" -max 60s
		# Per-thread CPU of the VMM mid-run: a saturated event-loop thread ("firecracker") means its
		# vsock muxer bounds the run; busy fc_vcpu threads mean the guest's CPU or its exits do.
		(sleep 10 && top -H -b -n 1 -d 1 -p "$vm" | sed -n '7,20p') >"${log%.log}.threads.txt" 2>&1 &
		local sampler=$!
		"$bench" --firecracker "$dir/v.sock:5000" "$callers" "$rounds" "${interval[@]}" >"$log"
		wait "$sampler" 2>/dev/null || true
		kill -KILL "$vm"
		wait "$vm" 2>/dev/null || true
		vm=''
	fi
	rm -rf "$dir"
}

for mode in closed paced; do
	mkdir -p "$out/$mode"
	for side in inproc guest; do run "$side" "$mode" "$out/$mode/warmup-$side.log"; done
	for pair in $(seq "$pairs"); do
		for side in inproc guest; do run "$side" "$mode" "$out/$mode/$side-$pair.log"; done
	done
done
python3 -I "$repo/scripts/m6-socket-summary.py" "$out" guest >"$out/summary.md"
cat "$out/summary.md"
