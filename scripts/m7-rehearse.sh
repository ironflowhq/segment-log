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
	M7_CACHE_TYPE=$1 "$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 &
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

echo "== VOL-FS-CHECK and VOL-FLUSH-CONFIG (Writeback)"
out=$(boot_once Writeback)
echo "$out"
grep -Eq "^m7: fs /dev/vdb ext4 " <<<"$out" || { echo "FAIL VOL-FS-CHECK" >&2; exit 1; }
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
rm -rf "$dir"
echo "refused while attached and while paused; attached after the owner died"

echo "== conformance over Firecracker vsock"
cd "$repo/clients/go"
SEGMENT_LOG_TRANSPORT=firecracker SEGMENT_LOG_SERVER="$repo/scripts/m7/fc-server" \
	go test -count=1 -run 'TestConformance|TestTransportUnknown|TestTransportRestart|TestHandoff' -v ./... |
	grep -E '^(=== RUN|--- |ok|FAIL|PASS)'
echo "== rehearsal passed"
