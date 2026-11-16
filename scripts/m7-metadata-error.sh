#!/usr/bin/env bash
# VOL-METADATA-ERROR (volume-requirements.md, V3) on the ADR 0059 stack, as root on the VMM host.
#
#   sudo M7_CACHE=<cache> scripts/m7-metadata-error.sh [--control | --panic]
#
# The guest kernel has no device-mapper, so the failing target sits under the volume on the host:
# a dm linear table over a loop device that the script switches to `error` and back. The guest
# keeps the qualified kernel, filesystem and mount options and sees the failures as I/O errors.
# The guest runs the four V3 steps (`powercut metaerr`) and waits at each switch on its console.
#
# Pass (ADR 0062): the directory fsync while writes fail returns an error, and every write after
# the device works again fails until remount. The later directory fsyncs are printed, and decide
# nothing. --control mounts with errors=continue, the mode V3 rules out:
# the case must fail under it, or the record states that this kernel stops writes anyway.
# --panic is V3's separate procedure for errors=panic: it passes when the guest halts at the
# failure, before any later call returns.
set -euo pipefail
control=${1:-}
case $control in "" | --control | --panic) ;; *) echo "unknown option $control" >&2; exit 64 ;; esac
[ "$(id -u)" = 0 ] || { echo "run as root: it uses losetup and dmsetup" >&2; exit 1; }
: "${M7_CACHE:?set M7_CACHE to the cache that scripts/m7/setup.sh built}"
repo=$(cd "$(dirname "$0")/.." && pwd)
dir=$(mktemp -d "${M7_WORK:-/tmp}/m7-v3-XXXX")
name=m7v3-$$
truncate -s 256M "$dir/backing.img"
loop=$(losetup -f --show "$dir/backing.img")
sectors=$(blockdev --getsz "$loop")
dmsetup create "$name" --table "0 $sectors linear $loop 0"
dev=/dev/mapper/$name
cleanup() {
	[ -n "${vm:-}" ] && kill -KILL "$vm" 2>/dev/null
	dmsetup remove "$name" 2>/dev/null || true
	losetup -d "$loop" 2>/dev/null || true
	rm -rf "$dir"
}
trap cleanup EXIT
mkfs.ext4 -q -F "$dev"
switch() { dmsetup suspend "$name" && dmsetup load "$name" --table "0 $sectors $1" && dmsetup resume "$name"; }

env=(M7_IMAGE="$dev")
# Both variants change a fingerprinted mount option on purpose (V10).
case $control in
--control) env+=(M7_GUEST_MOUNT=errors=continue M7_DRIFT_CHECK=0) ;;
--panic) env+=(M7_GUEST_MOUNT=errors=panic M7_DRIFT_CHECK=0) ;;
esac
mkfifo "$dir/serial"
env "${env[@]}" "$repo/scripts/m7/fc-server" --dir "$dir/log" --metaerr <>"$dir/serial" &
vm=$!
answered=
for _ in $(seq 600); do
	if grep -q "^m7: READY-FAIL" "$dir/console.log" 2>/dev/null && [ -z "$answered" ]; then
		switch error
		echo failing >"$dir/serial"
		answered=fail
	elif grep -q "^m7: READY-RECOVER" "$dir/console.log" 2>/dev/null && [ "$answered" = fail ]; then
		switch "linear $loop 0"
		echo working >"$dir/serial"
		answered=recover
	fi
	kill -0 "$vm" 2>/dev/null || break
	sleep 0.1
done
wait "$vm" || true
vm=
# The serial console ends lines with CR LF.
out=$(tr -d '\r' <"$dir/console.log" | grep "^m7: " | sed 's/^m7: //')
echo "$out"
if [ "$control" = --panic ]; then
	tr -d '\r' <"$dir/console.log" | grep -m1 "Kernel panic" || true
	if tr -d '\r' <"$dir/console.log" | grep -q "Kernel panic" && ! grep -q "^STEP 4-" <<<"$out"; then
		echo "== V3 errors=panic result: pass: the guest halted at the failure, before any later call returned"
		exit 0
	fi
	echo "== V3 errors=panic result: fail: the guest did not halt before a later call returned" >&2
	exit 1
fi
step() { grep "^STEP $1 " <<<"$out" | awk '{print $3}'; }
[ "$(step 1-file)" = ok ] || { echo "FAIL: step 1 did not make a durable file" >&2; exit 1; }
grep -q "^DONE" <<<"$out" || { echo "FAIL: the steps did not finish" >&2; exit 1; }
v3=pass
[ "$(step 2-dirsync)" = err ] || v3="fail: the directory fsync succeeded while writes failed"
[ "$(step 4-file)" = err ] || v3="fail: a write succeeded after the device recovered"
echo "evidence: directory fsync after recovery: held $(step 4-dirsync-held), new $(step 4-dirsync-new)"
echo "== V3 ${control:+control }result: $v3"
if [ "$control" = --control ]; then
	if [ "$v3" != pass ]; then
		echo "control: errors=continue fails the case (expected)"
	else
		echo "control: this kernel stops writes under errors=continue too, so no control could be built"
	fi
else
	[ "$v3" = pass ] || exit 1
fi
