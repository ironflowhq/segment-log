#!/usr/bin/env bash
# VOL-POWER-CUT-RECOVERY (volume-requirements.md, V2) on the ADR 0059 stack: a host kernel crash
# while open's rewrite of already-durable bytes is in flight to the drive (ADR 0015). Runs on a
# controller, like scripts/m7-host-cut.sh, and reads the evidence on the way:
#
#   M7_HOST=root@<host> M7_OBSERVER=<observer>:7070 \
#     scripts/m7-recovery-cut.sh clean|sealed|header [cycles=10] [--control]
#
# Fixtures: clean, the last group frame acknowledged and durable (it rarely ends on a sector
# boundary, so it shares sectors with earlier acknowledged bytes); sealed, a sealed log whose seal
# run open rewrites; header, a new log whose only segment holds just its header.
#
# Each cycle leaves the fixture on a volume behind a dm-delay device, then slows the device's
# writes (M7_DELAY_MS, default 3000) and boots the guest. Open reads, then rewrites, then syncs, so
# the rewrite reaches the drive only in that sync, and the delay holds it there. The host streams
# the guest console and its block trace to this controller. Once the console shows the guest past
# its mount and a write is queued with no completion, the controller crashes the host. A cycle
# counts only with that evidence, saved in the cycle's stream file. After the boot, the volume
# reopens without delay, and the cycle passes when open succeeds without Corrupt and every record
# the observer stored is there.
#
# --control boots the cut guest with the rewrite-control server, which writes one changed byte over
# an acknowledged sector. It crashes once the server answers, so open's sync of the changed byte has
# returned, and passes at the first lost record.
set -euo pipefail

fixture=${1:?usage: m7-recovery-cut.sh clean|sealed|header [cycles] [--control]}
cycles=${2:-10}
control=${3:-}
case $fixture in clean | sealed | header) ;; *) echo "unknown fixture $fixture" >&2; exit 64 ;; esac
case $control in "" | --control) ;; *) echo "unknown option $control" >&2; exit 64 ;; esac
: "${M7_HOST:?set M7_HOST}" "${M7_OBSERVER:?set M7_OBSERVER}"
repo=${M7_REPO:-/root/segment-log}
hc=$repo/scripts/m7/host-cycle
delay=${M7_DELAY_MS:-3000}
run=v2-$fixture-$(date +%Y%m%d%H%M%S)${control:+-control}
work=/tmp/m7-$run
out=${M7_OUT:-./m7-v2-out}/$run
mkdir -p "$out"

on_host() { ssh -o ConnectTimeout=10 -o BatchMode=yes -o ServerAliveInterval=2 -o ServerAliveCountMax=3 "$M7_HOST" "$@"; }
wait_for() {
	local deadline=$((SECONDS + $2))
	while [ $SECONDS -lt $deadline ]; do
		if on_host true 2>/dev/null; then [ "$1" = up ] && return 0; else [ "$1" = down ] && return 0; fi
		sleep 2
	done
	echo "FAIL: the host did not go $1 within $2 s" >&2
	exit 1
}

echo "== $run: $cycles cycles, delay $delay ms, host $M7_HOST, observer $M7_OBSERVER"
on_host "$hc" prep "$work" >/dev/null
lost=0
counted=0
for cycle in $(seq "$cycles"); do
	# A sealed log takes no more appends, and the header fixture needs a log with no record, so
	# both start from a new image. The clean fixture keeps one volume, and each check covers every
	# earlier cycle.
	fresh=
	if [ "$fixture" != clean ] || [ "$cycle" = 1 ]; then fresh=fresh; fi
	on_host "$hc" delay "$work" 0 $fresh
	on_host "$hc" prepare "$work" "$fixture" "$run" "$cycle" "$M7_OBSERVER"
	on_host "$hc" delay "$work" "$delay"
	before=$(on_host "$hc" boot-id)
	stream=$out/stream-$cycle.log
	on_host "$hc" stream "$work" >"$stream" 2>/dev/null &
	streamer=$!
	sleep 2 # let blktrace attach
	on_host "${control:+M7_REWRITE_CONTROL=1} $hc" boot "$work"
	# The evidence: the console line the guest prints after its mount, then a queued write (Q) with
	# no completion (C) yet. For the control: the server answers, so its open has synced.
	evidence=
	if [ "$control" = --control ] && on_host "$hc" ready "$work"; then
		evidence="the control server answered after open"
	fi
	for _ in $(seq 600); do
		[ "$control" = --control ] && break
		if grep -q "^CON m7: write_cache" "$stream"; then
			after_mount=$(sed -n '/^CON m7: write_cache/,$p' "$stream")
			queued=$(grep -m1 "^BLK Q W" <<<"$after_mount" || true)
			if [ -n "$queued" ]; then
				sector=$(awk '{print $4}' <<<"$queued")
				grep -q "^BLK C W[A-Z]* $sector " <<<"$after_mount" || evidence=$queued
			fi
		fi
		[ -n "$evidence" ] && break
		sleep 0.05
	done
	if [ -z "$evidence" ]; then
		echo "-- cycle $cycle: no rewrite write seen in flight; not counted"
		kill "$streamer" 2>/dev/null || true
		on_host "pkill -KILL -x firecracker" || true
		continue
	fi
	on_host 'echo c >/proc/sysrq-trigger' 2>/dev/null || true
	echo "EVIDENCE $evidence" >>"$stream"
	wait_for down 120
	wait_for up 900
	kill "$streamer" 2>/dev/null || true
	after=$(on_host "$hc" boot-id)
	[ "$before" != "$after" ] || { echo "FAIL: cycle $cycle: the host did not reboot" >&2; exit 1; }
	counted=$((counted + 1))
	on_host "$hc" delay "$work" 0
	only=()
	[ "$fixture" != clean ] && only=("$cycle")
	rc=0
	on_host "$hc" check "$work" "$run" "$M7_OBSERVER" "${only[@]}" >"$out/check-$cycle.log" 2>&1 || rc=$?
	if [ "$rc" != 0 ] && [ "$rc" != 1 ] && on_host "grep -q Corrupt $work/console.log"; then
		rc=1 # open failed with Corrupt: a failed cycle, not a broken harness
	fi
	case $rc in 0) ;; 1) lost=$((lost + 1)) ;; *) echo "FAIL: cycle $cycle: check exit $rc" >&2; exit 1 ;; esac
	echo "-- cycle $cycle: $(tail -n 1 "$out/check-$cycle.log"); evidence: $evidence"
	[ "$control" = --control ] && [ $lost -gt 0 ] && break
done

echo "== $run: $lost of $counted counted cycles failed (open Corrupt or a reported record missing)"
if [ "$control" = --control ]; then
	[ "$lost" -gt 0 ] || { echo "FAIL: the control lost nothing" >&2; exit 1; }
	echo "control detected (expected)"
else
	[ "$counted" -ge "$cycles" ] || echo "note: only $counted of $cycles cycles counted; run more" >&2
	[ "$lost" -eq 0 ] || { echo "FAIL: V2" >&2; exit 1; }
	echo "pass: $counted counted cycles, zero records missing, open never Corrupt"
fi
