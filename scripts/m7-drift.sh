#!/usr/bin/env bash
# VOL-DRIFT (volume-requirements.md, V10) on the ADR 0059 stack: change one fingerprint field at a
# time and check that the log does not open and the error names the field.
#
#   scripts/m7-drift.sh
#
# Cases: the unchanged stack opens; vmm.cache_type (Unsafe) stops the launcher; guest.fs (an added
# mount option) and guest.kernel (another guest kernel) stop the guest before the server starts.
set -euo pipefail
# shellcheck source=m7/setup.sh
source "$(dirname "$0")/m7/setup.sh"
other=${M7_DRIFT_KERNEL:-5.10.245}
[ -s "$M7_CACHE/vmlinux-$other" ] ||
	curl -fsSL -o "$M7_CACHE/vmlinux-$other" "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/$ci/$arch/vmlinux-$other"

# boot <expected field or "none"> [env...]: boots one guest and checks what stopped it.
boot() {
	local want=$1 dir rc=0
	shift
	dir=$(mktemp -d /tmp/m7-drift-XXXX)
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
boot guest.fs M7_GUEST_MOUNT=noatime
boot guest.kernel "M7_VMLINUX=$M7_CACHE/vmlinux-$other"
echo "== drift cases passed"
