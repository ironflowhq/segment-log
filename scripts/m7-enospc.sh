#!/usr/bin/env bash
# VOL-ENOSPC (volume-requirements.md, V9) on the ADR 0059 stack, as root on the VMM host.
#
#   sudo M7_CACHE=<cache> scripts/m7-enospc.sh guest|host
#
# guest: the guest filesystem fills. The volume image is small; after the load stops, the script
#   grows the image and the filesystem in it.
# host: the host filesystem under the sparse image fills first. The image lives on a small
#   loop-mounted ext4; after the load stops, the script grows that filesystem online.
# Then the same volume reopens and every record the load reported must be there, at its offset and
# revision, with no Corrupt. The load reports to an observer this script runs on the host: no
# machine goes down in this case, so it need not be another machine.
set -euo pipefail
layer=${1:?usage: m7-enospc.sh guest|host}
case $layer in guest | host) ;; *) echo "unknown layer $layer" >&2; exit 64 ;; esac
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
: "${M7_CACHE:?set M7_CACHE to the cache that scripts/m7/setup.sh built}"
repo=$(cd "$(dirname "$0")/.." && pwd)
pc=$M7_CACHE/powercut
work=$(mktemp -d "${M7_WORK:-/tmp}/m7-enospc-XXXX")
run=enospc-$layer-$(date +%Y%m%d%H%M%S)
vm='' observer='' loop=''
cleanup() {
	set +e # every step runs, whatever the one before returned
	[ -n "$vm" ] && { kill -KILL "$vm" 2>/dev/null; wait "$vm" 2>/dev/null; }
	[ -n "$observer" ] && kill "$observer" 2>/dev/null
	mountpoint -q "$work/hostfs" 2>/dev/null && umount "$work/hostfs"
	[ -n "$loop" ] && losetup -d "$loop" 2>/dev/null
	rm -rf "$work"
}
trap cleanup EXIT
"$pc" observe -listen 127.0.0.1:0 -dir "$work/reports" >/dev/null 2>"$work/observer.log" &
observer=$!
for _ in $(seq 50); do grep -q observing "$work/observer.log" && break; sleep 0.1; done
address=$(sed -n 's/.*observing on //p' "$work/observer.log")

dir=$work
if [ "$layer" = host ]; then
	truncate -s 160M "$work/hostfs.img"
	loop=$(losetup -f --show "$work/hostfs.img")
	mkfs.ext4 -q "$loop"
	mkdir "$work/hostfs"
	mount "$loop" "$work/hostfs"
	dir=$work/hostfs
	export M7_LOG_BYTES=1G # sparse: the host filesystem fills long before the guest's
else
	export M7_LOG_BYTES=96M
fi
payload=4096
vmm() { "$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 </dev/null >>"$work/vmm.log" 2>&1 & vm=$!; }

echo "== $run: fill the $layer filesystem during load"
vmm
"$pc" ready -address "$dir/v.sock:5000"
# Unpaced, with large payloads, so the layer fills in seconds. The load ends when appends fail.
"$pc" load -address "$dir/v.sock:5000" -observer "$address" -run "$run" -cycle 1 \
	-interval 0 -payload "$payload" -max 120s || true
[ "$layer" = host ] && df -h "$dir" | tail -n 1
kill -KILL "$vm" 2>/dev/null || true
wait "$vm" 2>/dev/null || true
vm=
grep -E "^m7: server exit" "$dir/console.log" | sort | uniq -c || true

echo "== restore capacity"
if [ "$layer" = host ]; then
	truncate -s 2G "$work/hostfs.img"
	losetup -c "$loop"
	resize2fs -f "$loop" >/dev/null
else
	e2fsck -fy "$dir/log.img" >/dev/null 2>&1 || true
	truncate -s 512M "$dir/log.img"
	resize2fs -f "$dir/log.img" >/dev/null
fi
[ "$layer" = host ] && df -h "$dir" | tail -n 1

echo "== reopen and check"
vmm
"$pc" ready -address "$dir/v.sock:5000"
rc=0
"$pc" check -address "$dir/v.sock:5000" -observer "$address" -run "$run" -payload "$payload" || rc=$?
case $rc in
0) echo "pass: $layer filesystem full: every acknowledged record survived, and open succeeded" ;;
*) echo "FAIL: check exit $rc" >&2; exit 1 ;;
esac
