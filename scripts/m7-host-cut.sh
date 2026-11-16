#!/usr/bin/env bash
# The power-cut harness (volume-requirements.md#power-cut-harness) for the host-side events of V7
# on the ADR 0059 stack. The event takes the VMM host down, so this runs on a third machine, the
# controller, and drives the host over ssh:
#
#   powercut observe -listen :7070 -dir <store>        on the observer, neither host nor controller
#   M7_HOST=root@<host> M7_OBSERVER=<observer>:7070 \
#     scripts/m7-host-cut.sh host-kernel-crash|host-power-loss [cycles] [--control]
#
# host-kernel-crash: `echo c > /proc/sysrq-trigger` on the host; kernel.panic reboots it.
# host-power-loss: the Robot web service presses the power button long, then again, so the host
#   loses power and boots. Needs M7_ROBOT_CURLRC (a curl config with `user = "<ws user>:<pass>"`)
#   and M7_ROBOT_SERVER (the server number). Volume-requirements.md counts it as a power cut only
#   with evidence that it removes power from the drives; record what the hardware does.
#
# The host needs the repo at M7_REPO (default /root/segment-log) and scripts/m7/setup.sh run once.
# Each cycle starts a load on new streams, causes the event at a uniformly random time in the load
# window, waits for the host to boot, recovers the same volume and checks every record the observer
# stored. A cycle counts only if the host's boot id changed. --control runs the guest with
# `cache_type: Unsafe`, so the host never flushes the image: the control passes when a cycle loses
# a reported record, and stops at the first one.
set -euo pipefail

event=${1:?usage: m7-host-cut.sh host-kernel-crash|host-power-loss [cycles] [--control]}
cycles=${2:-30}
control=${3:-}
case $event in host-kernel-crash | host-power-loss) ;; *) echo "unknown event $event" >&2; exit 64 ;; esac
case $control in "" | --control) ;; *) echo "unknown option $control" >&2; exit 64 ;; esac
: "${M7_HOST:?set M7_HOST to the VMM host, as user@address}"
: "${M7_OBSERVER:?set M7_OBSERVER to the observer host:port}"
if [ "$event" = host-power-loss ]; then
	: "${M7_ROBOT_CURLRC:?set M7_ROBOT_CURLRC}" "${M7_ROBOT_SERVER:?set M7_ROBOT_SERVER}"
fi
repo=${M7_REPO:-/root/segment-log}
window_ms=${M7_WINDOW_MS:-3000}
cache=Writeback
drift_check=1
# The control changes the stack on purpose, so it skips the fingerprint check (V10).
[ "$control" = --control ] && cache=Unsafe drift_check=0
run=hc-$event-$(date +%Y%m%d%H%M%S)${control:+-control}
work=/tmp/m7-$run

on_host() { ssh -o ConnectTimeout=10 -o BatchMode=yes -o ServerAliveInterval=2 -o ServerAliveCountMax=3 "$M7_HOST" "$@"; }
robot() {
	curl -fsS -K "$M7_ROBOT_CURLRC" "https://robot-ws.your-server.de/reset/$M7_ROBOT_SERVER" -d "type=$1" >/dev/null
}
wait_for() { # wait_for up|down <seconds>
	local deadline=$((SECONDS + $2))
	while [ $SECONDS -lt $deadline ]; do
		if on_host true 2>/dev/null; then [ "$1" = up ] && return 0; else [ "$1" = down ] && return 0; fi
		sleep 2
	done
	echo "FAIL: the host did not go $1 within $2 s" >&2
	exit 1
}

echo "== $run: $cycles cycles, host $M7_HOST, observer $M7_OBSERVER, cache_type $cache"
on_host "$repo/scripts/m7/host-cycle" prep "$work"
lost=0
counted=0
for cycle in $(seq "$cycles"); do
	before=$(on_host "$repo/scripts/m7/host-cycle" boot-id)
	on_host "M7_CACHE_TYPE=$cache M7_DRIFT_CHECK=$drift_check $repo/scripts/m7/host-cycle" start "$work" "$run" "$cycle" "$M7_OBSERVER"
	delay=$((500 + RANDOM % window_ms))
	sleep "$(printf '%d.%03d' $((delay / 1000)) $((delay % 1000)))"
	case $event in
	host-kernel-crash) on_host 'echo c >/proc/sysrq-trigger' 2>/dev/null || true ;;
	host-power-loss)
		robot power_long
		wait_for down 120
		sleep 10 # let the power supply drain before it comes back
		robot power
		;;
	esac
	wait_for down 120
	wait_for up 900
	after=$(on_host "$repo/scripts/m7/host-cycle" boot-id)
	[ "$before" != "$after" ] || { echo "FAIL: cycle $cycle: the host did not reboot" >&2; exit 1; }
	counted=$((counted + 1))
	only=()
	[ "$control" = --control ] && only=("$cycle")
	# check exits 1 for a missing record; anything else is the harness failing, never a pass.
	rc=0
	on_host "$repo/scripts/m7/host-cycle" check "$work" "$run" "$M7_OBSERVER" "${only[@]}" || rc=$?
	case $rc in 0) ;; 1) lost=$((lost + 1)) ;; *) echo "FAIL: cycle $cycle: check exit $rc" >&2; exit 1 ;; esac
	echo "-- cycle $cycle at ${delay} ms"
	[ "$control" = --control ] && [ $lost -gt 0 ] && break
done

echo "== $run: $lost of $counted cycles missed a reported record"
if [ "$control" = --control ]; then
	[ "$lost" -gt 0 ] || { echo "FAIL: the control lost nothing, so the cut does not drop the host's caches" >&2; exit 1; }
	echo "control detected (expected)"
else
	[ "$lost" -eq 0 ] || { echo "FAIL: acknowledged records were lost" >&2; exit 1; }
	echo "pass: zero acknowledged records missing in $counted cycles"
fi
