#!/usr/bin/env bash
# The power-cut harness (volume-requirements.md#power-cut-harness) on the ADR 0059 stack, for the
# guest-side events of V7. Runs on the VMM host, with an observer on a machine the event does not
# touch:
#
#   powercut observe -listen :7070 -dir <store>                         (the other machine)
#   M7_OBSERVER=<host>:7070 scripts/m7-power-cut.sh <event> [cycles] [--control]
#
# event: process-crash (SIGKILL the server in the guest), guest-crash (sysrq c in the guest), or
# guest-destroy (SIGKILL the VMM). Each cycle starts a load on new streams, causes the event at a
# uniformly random time in the load window, brings the server back on the same volume (recovery),
# and checks every record the observer stored. Cycles run on one volume, so each check covers
# every earlier cycle too.
#
# --control runs the known-bad control: the load reports each record before its acknowledgment.
# The control passes when at least one cycle misses a reported record, which shows the harness
# can see a loss; without --control, the run passes when no cycle misses one.
#
# Host kernel crash and host power loss need the dedicated host and a second machine to cause
# them; they are not in this script.
set -euo pipefail

event=${1:?usage: m7-power-cut.sh process-crash|guest-crash|guest-destroy [cycles] [--control]}
cycles=${2:-30}
control=${3:-}
case $event in process-crash | guest-crash | guest-destroy) ;; *) echo "unknown event $event" >&2; exit 64 ;; esac
case $control in "" | --control) ;; *) echo "unknown option $control" >&2; exit 64 ;; esac
: "${M7_OBSERVER:?set M7_OBSERVER to the observer host:port, on another machine}"
window_ms=${M7_WINDOW_MS:-3000} # the event lands uniformly in [500, 500 + window) ms of load
export M7_LOG_BYTES=${M7_LOG_BYTES:-8G} # sparse; 30 cycles of the default load stay well inside

# shellcheck source=m7/setup.sh
source "$(dirname "$0")/m7/setup.sh"
pc=$M7_CACHE/powercut
early=()
[ "$control" = --control ] && early=(-report-early)
run=pc-$event-$(date +%Y%m%d%H%M%S)${control:+-control}
dir=$(mktemp -d /tmp/m7-pc-XXXX)
mkfifo "$dir/serial"
address=$dir/v.sock:5000
vm=

start_vm() {
	# Read and write, so opening the FIFO never blocks and the guest's serial input stays open.
	"$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 <>"$dir/serial" &
	vm=$!
	"$pc" ready -address "$address"
}
cleanup() {
	[ -n "$vm" ] && kill -KILL "$vm" 2>/dev/null
	rm -rf "$dir"
}
trap cleanup EXIT

echo "== $run: $cycles cycles, observer $M7_OBSERVER"
start_vm
lost_cycles=0
for cycle in $(seq "$cycles"); do
	"$pc" load -address "$address" -observer "$M7_OBSERVER" -run "$run" -cycle "$cycle" "${early[@]}" &
	loader=$!
	delay=$((500 + RANDOM % window_ms))
	sleep "$(printf '%d.%03d' $((delay / 1000)) $((delay % 1000)))"
	case $event in
	process-crash) echo kill >"$dir/serial" ;; # the guest's supervisor restarts the server
	guest-crash)
		echo crash >"$dir/serial"
		wait "$vm" || true # panic=1 reboots, and the VMM exits
		start_vm
		;;
	guest-destroy)
		kill -KILL "$vm"
		wait "$vm" || true
		start_vm
		;;
	esac
	wait "$loader" || { echo "FAIL: the load in cycle $cycle failed" >&2; exit 1; }
	"$pc" ready -address "$address"
	# A real run checks every cycle's reports each time; the control counts each cycle's own.
	only=()
	[ "$control" = --control ] && only=(-cycle "$cycle")
	# check exits 1 for a missing record; anything else is the harness failing, never a pass.
	rc=0
	"$pc" check -address "$address" -observer "$M7_OBSERVER" -run "$run" "${only[@]}" || rc=$?
	case $rc in 0) ;; 1) lost_cycles=$((lost_cycles + 1)) ;; *) echo "FAIL: cycle $cycle: check exit $rc" >&2; exit 1 ;; esac
	echo "-- cycle $cycle at ${delay} ms"
done

echo "== $run: $lost_cycles of $cycles cycles missed a reported record"
if [ "$control" = --control ]; then
	[ "$lost_cycles" -gt 0 ] || { echo "FAIL: the control lost nothing, so the harness cannot see a loss" >&2; exit 1; }
	echo "control detected (expected)"
else
	[ "$lost_cycles" -eq 0 ] || { echo "FAIL: acknowledged records were lost" >&2; exit 1; }
	echo "pass: zero acknowledged records missing in $cycles cycles"
fi
