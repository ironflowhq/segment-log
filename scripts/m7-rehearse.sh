#!/usr/bin/env bash
# M7 stage 1 (ADR 0059): the Firecracker stack end to end on any Linux host with /dev/kvm. On the
# Mac that host is a Lima VM with nested virtualization; on the dedicated host it is the host.
# A pass here fills no qualification row and reports no benchmark.
#
#   scripts/m7-rehearse.sh
#
# After scripts/m7/setup.sh, it checks:
#   VOL-FS-CHECK      the log directory is on ext4 on a block device, in the guest
#   VOL-FLUSH-CONFIG  Writeback shows the guest a write-back cache (flushes reach the host);
#                     the Unsafe control shows write through (no flush feature)
#   conformance       the Go conformance and transport cases over Firecracker vsock
#   HANDOFF           each start is a new guest on the same volume: a sealed log and a crashed
#                     one are read back by a --read-only server on a read-only drive (ADR 0060)
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=m7/setup.sh
source "$(dirname "$0")/m7/setup.sh"

# boot_once <cache type>: boots one guest until it prints its write_cache line, then destroys it.
boot_once() {
	local dir
	dir=$(mktemp -d /tmp/m7-XXXX)
	# The Unsafe control changes the stack on purpose, so it skips the fingerprint check (V10).
	M7_CACHE_TYPE=$1 M7_DRIFT_CHECK=$([ "$1" = Writeback ] && echo 1 || echo 0) \
		"$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 &
	local pid=$!
	for _ in $(seq 100); do
		grep -q "m7: write_cache" "$dir/console.log" 2>/dev/null && break
		sleep 0.1
	done
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	grep "^m7:" "$dir/console.log" || true
	rm -rf "$dir"
}

# V4: the log directory is on ext4 or xfs, and the source is a block device.
fs_ok() { # fs_ok "<source> <type> <options>"
	local source type
	read -r source type _ <<<"$1"
	case $type in ext4 | xfs) [ -b "$source" ] || [[ $source == /dev/vd* ]] ;; *) return 1 ;; esac
}

echo "== VOL-FS-CHECK and VOL-FLUSH-CONFIG (Writeback)"
out=$(boot_once Writeback)
echo "$out"
fs_ok "$(sed -n 's/^m7: fs //p' <<<"$out")" || { echo "FAIL VOL-FS-CHECK" >&2; exit 1; }
# The control: the same rule rejects a directory on tmpfs.
shm=$(findmnt -no SOURCE,FSTYPE,OPTIONS -T /dev/shm | tr -s ' ')
! fs_ok "$shm" || { echo "FAIL VOL-FS-CHECK control: accepted $shm" >&2; exit 1; }
echo "control: rejects /dev/shm ($shm)"
grep -q "^m7: write_cache write back" <<<"$out" || { echo "FAIL VOL-FLUSH-CONFIG" >&2; exit 1; }

echo "== VOL-FLUSH-CONFIG control (Unsafe)"
out=$(boot_once Unsafe)
echo "$out"
grep -q "^m7: write_cache write through" <<<"$out" || { echo "FAIL control: Unsafe did not hide the flush feature" >&2; exit 1; }

echo "== one VMM per volume (VOL-EXCLUSIVE steps 2 and 4, one host)"
dir=$(mktemp -d /tmp/m7-XXXX)
"$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 &
pid=$!
sleep 2
fence() { "$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 2>/dev/null && echo 0 || echo $?; }
[ "$(fence)" = 75 ] || { echo "FAIL: a second VMM attached the volume" >&2; exit 1; }
kill -STOP "$pid"
[ "$(fence)" = 75 ] || { echo "FAIL: a VMM attached the volume while its owner was paused" >&2; exit 1; }
kill -KILL "$pid"
wait "$pid" 2>/dev/null || true
rc=0
timeout 3 "$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 || rc=$?
[ "$rc" = 124 ] || { echo "FAIL: no VMM could attach after the owner died (exit $rc)" >&2; exit 1; }
# The control: without the fence, a second VMM attaches the volume its owner still holds.
"$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 &
pid=$!
sleep 2
rc=0
M7_FENCE=0 timeout 3 "$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5001 2>/dev/null || rc=$?
kill -KILL "$pid"
wait "$pid" 2>/dev/null || true
[ "$rc" = 124 ] || { echo "FAIL VOL-EXCLUSIVE control: an unfenced launcher did not attach (exit $rc)" >&2; exit 1; }
rm -rf "$dir"
echo "refused while attached and while paused; attached after the owner died"
echo "control: with the fence off, a second VMM attached the held volume"

echo "== two launches at once (VOL-EXCLUSIVE step 3, one host)"
# Two launchers race for one unattached volume, 20 times: exactly one may boot it.
for round in $(seq 20); do
	dir=$(mktemp -d /tmp/m7-XXXX)
	truncate -s 64M "$dir/log.img" && /sbin/mkfs.ext4 -q -F "$dir/log.img"
	for _ in 1 2; do
		(
			rc=0
			timeout 3 "$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 2>/dev/null || rc=$?
			echo "$rc" >>"$dir/codes"
		) &
	done
	wait
	mapfile -t codes <"$dir/codes"
	rm -rf "$dir"
	# 124: booted and ran until the timeout; 75: refused by the fence.
	[ "$(printf '%s\n' "${codes[@]}" | sort | tr '\n' ' ')" = "124 75 " ] ||
		{ echo "FAIL: round $round: launch exit codes ${codes[*]}, want one 124 and one 75" >&2; exit 1; }
done
echo "exactly one of two simultaneous launches attached, in 20 of 20 rounds"

echo "== conformance over Firecracker vsock"
cd "$repo/clients/go"
SEGMENT_LOG_TRANSPORT=firecracker SEGMENT_LOG_SERVER="$repo/scripts/m7/fc-server" \
	go test -count=1 -run 'TestConformance|TestTransportUnknown|TestTransportRestart|TestHandoff' -v ./... |
	grep -E '^(=== RUN|--- |ok|FAIL|PASS)'
echo "== rehearsal passed"
