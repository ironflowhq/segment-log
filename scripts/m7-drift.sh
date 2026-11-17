#!/usr/bin/env bash
# VOL-DRIFT (volume-requirements.md, V10) on the ADR 0059 stack: change one fingerprint field at a
# time and check that the log does not open and the error names the field.
#
#   scripts/m7-drift.sh
#
# Cases: the unchanged stack opens; vmm.cache_type (Unsafe), host.fs (the image directory bind-mounted
# with an added mount option) and host.drive (a fingerprint that qualified another drive, as on a
# swapped machine) stop the launcher; guest.fs (an added mount option) and guest.kernel (another
# guest kernel) stop the guest before the server starts. The unchanged stack is the control for
# every case: it passes the same checks, host keys included.
#
# Before qualification the host keys in scripts/m7/fingerprint are `unpinned`; the script then pins
# this host's values in a copy, so that the host cases run anywhere.
set -euo pipefail
# shellcheck source=m7/setup.sh
source "$(dirname "$0")/m7/setup.sh"
other=${M7_DRIFT_KERNEL:-5.10.245}
fetch "vmlinux-$other" "vmlinux-$other"
pinned=$(mktemp /tmp/m7-fingerprint-XXXX)
trap 'rm -f "$pinned" "$pinned.other"' EXIT
grep -v '^host\.' "$repo/scripts/m7/fingerprint" >"$pinned"
if grep -q '^host\..*=unpinned$' "$repo/scripts/m7/fingerprint"; then
	"$repo/scripts/m7/fingerprint-host" /tmp >>"$pinned"
	echo "host keys unpinned: checking this host's values"
else
	grep '^host\.' "$repo/scripts/m7/fingerprint" >>"$pinned"
fi
export M7_FINGERPRINT=$pinned
sed 's/^host\.drive=.*/host.drive=Another Drive 0000/' "$pinned" >"$pinned.other"

# boot <expected field or "none"> [env...]: boots one guest and checks what stopped it. With BIND
# set, the guest's directory is first bind-mounted on itself with that mount option added.
boot() {
	local want=$1 dir rc=0
	shift
	dir=$(mktemp -d /tmp/m7-drift-XXXX)
	if [ -n "${BIND:-}" ]; then
		sudo mount --bind "$dir" "$dir"
		sudo mount -o "remount,bind,$BIND" "$dir"
	fi
	env "$@" timeout 20 "$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 2>"$dir/err" &
	local vm=$!
	if [ "$want" = none ]; then
		"$M7_CACHE/powercut" ready -address "$dir/v.sock:5000" ||
			{ echo "FAIL: the unchanged stack did not open the log" >&2; exit 1; }
		kill "$vm"
		wait "$vm" 2>/dev/null || true
		echo "unchanged: the log opened"
	else
		wait "$vm" || rc=$?
		[ -z "${BIND:-}" ] || sudo umount "$dir"
		if grep -q "drift: $want " "$dir/err" "$dir/console.log" 2>/dev/null &&
			! grep -q "^m7: server exit" "$dir/console.log" 2>/dev/null; then
			echo "$want: refused (exit $rc): $(grep -h "drift: $want " "$dir/err" "$dir/console.log" 2>/dev/null | head -n 1)"
		else
			echo "FAIL: changing $want did not stop the log with an error naming it" >&2
			cat "$dir/err" "$dir/console.log" >&2 2>/dev/null || true
			exit 1
		fi
	fi
	rm -rf "$dir"
}

echo "== VOL-DRIFT"
boot none
boot vmm.cache_type M7_CACHE_TYPE=Unsafe
BIND=noexec boot host.fs
boot host.drive "M7_FINGERPRINT=$pinned.other"
boot guest.fs M7_GUEST_MOUNT=noatime
boot guest.kernel "M7_VMLINUX=$M7_CACHE/vmlinux-$other"
echo "== drift cases passed"
