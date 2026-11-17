#!/usr/bin/env bash
# VOL-FLUSH-LATENCY (volume-requirements.md, V11) on the ADR 0059 stack. Boots `guests` guests at
# once, each running the flush probe (`powercut probe`, in place of the server) on its own volume
# image, and prints each guest's cycle statistics and the host drive's flushes per second.
#
#   scripts/m7-flush-latency.sh [guests=1] [rate=0] [seconds=60]
#
# rate is each guest's cycles per second, 0 for no limit. Run it three times: alone (1 guest), at
# the planned density with the planned group rate, and at that density with no limit, as a
# saturation diagnostic. M7_GROUP_BYTES is the expected encoded group (default 16384). The images
# go under M7_WORK (default /tmp), which must be the qualified host filesystem on a block device.
set -euo pipefail
guests=${1:-1}
rate=${2:-0}
seconds=${3:-60}
bytes=${M7_GROUP_BYTES:-16384}
export M7_LOG_BYTES=${M7_LOG_BYTES:-4G} M7_VCPUS=${M7_VCPUS:-1}
# shellcheck source=m7/setup.sh
source "$(dirname "$0")/m7/setup.sh"
root=$(mktemp -d "${M7_WORK:-/tmp}/m7-fl-XXXX")
source=$(findmnt -no SOURCE -T "$root" | tail -n 1) # the top mount, if /tmp is stacked
[ -b "$source" ] || { echo "$root is on $source, not a block device" >&2; rm -rf "$root"; exit 1; }
disk=$(lsblk -no PKNAME "$source" | head -n 1)
disk=${disk:-$(basename "$source")}
# Field 16 of the block stat file counts completed flush requests (Linux 5.5 and later).
flushes() { awk '{print $16}' "/sys/block/$disk/stat"; }

echo "== VOL-FLUSH-LATENCY: $guests guest(s), rate ${rate}/s each, ${seconds} s, ${bytes}-byte writes, host disk $disk"
before=$(flushes)
began=$SECONDS
pids=()
for i in $(seq "$guests"); do
	mkdir -p "$root/$i"
	"$repo/scripts/m7/fc-server" --dir "$root/$i/log" \
		--probe -bytes "$bytes" -duration "${seconds}s" -rate "$rate" &
	pids+=($!)
done
wait "${pids[@]}"
after=$(flushes)
elapsed=$((SECONDS - began))
failed=0
for i in $(seq "$guests"); do
	line=$(grep -h "^m7: PROBE" "$root/$i/console.log" || true)
	if [ -n "$line" ]; then echo "guest $i: ${line#m7: }"; else echo "guest $i: no PROBE line" >&2; failed=1; fi
done
echo "host: $((after - before)) flushes in ${elapsed} s (boot and warm-up included), $(((after - before) / (elapsed > 0 ? elapsed : 1))) per second"
rm -rf "$root"
exit "$failed"
