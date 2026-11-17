#!/usr/bin/env bash
# VOL-CAPACITY (volume-requirements.md, V9) on the ADR 0059 stack, as root on the VMM host.
#
#   sudo M7_CACHE=<cache> scripts/m7-capacity.sh [m7-capacity.py arguments]
#
# The arguments override the provisional workload profile, which is m7-capacity.py's defaults and
# --history-bytes 50GiB (qualification.md, Provisional values). M7_WORK (default /tmp) must be on
# the host filesystem that holds the log images.
#
# 1. Allocation ratio: a guest writes a test log and seals it; the ratio is the segment files'
#    allocated size (du) over their encoded size (their length).
# 2. Sizing record: m7-capacity.py with that ratio. A sparse image of the needed guest free space is
#    formatted as the launcher does, to measure df avail over image size; then an image of the
#    computed size must show at least the needed free bytes after mkfs (the guest layer), and the
#    host filesystem must have room for every log's whole image plus its alert threshold (the
#    host layer: the images are sparse, so each allocates at most its size).
# 3. Fill test, guest layer: the guest's alert (scripts/m7/capacity-alert) watches a small volume
#    while the load fills it.
# 4. Fill test, host layer: the host's alert watches a small host filesystem under the sparse image
#    while the load fills it.
# A fill passes when the alert was quiet at or above its threshold, fired below it, and fired while
# the layer still had free bytes. The fills use small layers and scaled thresholds: the alert is the
# same comparison at any size, and step 2 records the real thresholds.
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
: "${M7_CACHE:?set M7_CACHE to the cache that scripts/m7/setup.sh built}"
repo=$(cd "$(dirname "$0")/.." && pwd)
pc=$M7_CACHE/powercut
args=("$@")
[ ${#args[@]} -gt 0 ] || args=(--history-bytes 50GiB)
work=$(mktemp -d "${M7_WORK:-/tmp}/m7-capacity-XXXX")
run=capacity-$(date +%Y%m%d%H%M%S)
vm='' observer='' loop='' alert=''
cleanup() {
	set +e # every step runs, whatever the one before returned
	[ -n "$vm" ] && { kill -KILL "$vm" 2>/dev/null; wait "$vm" 2>/dev/null; }
	[ -n "$alert" ] && kill "$alert" 2>/dev/null
	[ -n "$observer" ] && kill "$observer" 2>/dev/null
	for m in "$work"/mnt "$work"/hostfs; do mountpoint -q "$m" 2>/dev/null && umount "$m"; done
	[ -n "$loop" ] && losetup -d "$loop" 2>/dev/null
	rm -rf "$work"
}
trap cleanup EXIT
"$pc" observe -listen 127.0.0.1:0 -dir "$work/reports" >/dev/null 2>"$work/observer.log" &
observer=$!
for _ in $(seq 50); do grep -q observing "$work/observer.log" && break; sleep 0.1; done
address=$(sed -n 's/.*observing on //p' "$work/observer.log")
mkdir "$work/mnt"
failed=0

# vmm <dir> [env...]: boots a guest on <dir>/log.img and waits until its server answers.
vmm() {
	local dir=$1
	shift
	mkdir -p "$dir"
	env "$@" "$repo/scripts/m7/fc-server" --dir "$dir/log" --vsock-port 5000 </dev/null >>"$work/vmm.log" 2>&1 &
	vm=$!
	"$pc" ready -address "$dir/v.sock:5000"
}
stop() { kill -KILL "$vm" 2>/dev/null || true; wait "$vm" 2>/dev/null || true; vm=; }
# load <dir> <cycle> [load flags]: appends of 4 KiB payloads; unpaced, they fill the volume.
load() {
	local dir=$1 cycle=$2
	shift 2
	"$pc" load -address "$dir/v.sock:5000" -observer "$address" -run "$run" -cycle "$cycle" \
		-payload 4096 "$@" || true
}
# df_bytes <field> <path>: one df field in bytes.
df_bytes() { df -B1 --output="$1" "$2" | tail -n 1 | tr -d ' '; }
# formatted <bytes>: df avail of a sparse image of that size, formatted as the launcher does.
formatted() {
	rm -f "$work/size.img"
	truncate -s "$1" "$work/size.img"
	/sbin/mkfs.ext4 -q -F "$work/size.img"
	mount -o ro,loop "$work/size.img" "$work/mnt"
	df_bytes avail "$work/mnt"
	umount "$work/mnt"
	rm -f "$work/size.img"
}
calc() { python3 "$repo/scripts/m7-capacity.py" "${args[@]}" "$@"; }
get() { sed -n "s/^$1=//p" <<<"$2"; }
# judge <layer> <threshold> <sample lines>: the fill test's verdict from capacity-alert's samples.
judge() {
	tr -d '\r' | awk -v layer="$1" -v t="$2" '
		$2 == "capacity" && $3 == layer {
			free = $5; n++
			if ($8 == "ok" && free < t || $8 == "alert" && free >= t) wrong++
			if ($8 == "ok" && !fired) last_ok = free
			if ($8 == "alert" && !fired) { fired = 1; first = free; at = n }
		}
		END {
			printf "%s alert: %d samples; last quiet at %s free, fired at %s free (threshold %s), %d samples before the last\n",
				layer, n, last_ok, first, t, n - at
			exit !(n && !wrong && last_ok != "" && last_ok >= t && fired && first > 0)
		}'
}

echo "== allocation ratio: a sealed test log"
# Paced for a few hundred MiB: several segments, far from full.
vmm "$work/ratio" M7_LOG_BYTES=4G
load "$work/ratio" 1 -max 10s
"$pc" seal -address "$work/ratio/v.sock:5000"
wait "$vm" 2>/dev/null || true
vm=
mount -o ro,loop "$work/ratio/log.img" "$work/mnt"
encoded=$(du -cb --apparent-size "$work"/mnt/log/*.seg | tail -n 1 | cut -f1)
allocated=$(du -cB1 "$work"/mnt/log/*.seg | tail -n 1 | cut -f1)
segments=$(find "$work/mnt/log" -name '*.seg' | wc -l)
umount "$work/mnt"
ratio=$(python3 -c "print(round($allocated / $encoded, 6))")
echo "test log: $segments segments, $encoded bytes encoded, $allocated allocated: ratio $ratio"

echo "== sizing record"
first=$(calc --ratio "$ratio")
probe=$(get guest_free_needed_bytes "$first")
avail=$(formatted "$probe")
avail_ratio=$(python3 -c "print(round($avail / $probe, 6))")
host_free=$(df_bytes avail "$work")
record=$(calc --ratio "$ratio" --avail-ratio "$avail_ratio" --host-free "$host_free") || true
echo "$record"
image=$(get image_bytes "$record")
need=$(get guest_free_needed_bytes "$record")
guest_free=$(formatted "$image")
echo "guest layer: df avail $guest_free after mkfs of a $image-byte image; needs $need" \
	"(planned allocated $(get planned_allocated_bytes_per_log "$record") + alert threshold $(get guest_alert_threshold_bytes "$record"))"
[ "$guest_free" -ge "$need" ] || { echo "FAIL: the guest volume is too small" >&2; failed=1; }
echo "host layer: df $(df -B1 --output=source,fstype,size,avail "$work" | tail -n 1 | tr -s ' '); needs" \
	"$(get host_free_needed_bytes "$record") ($(get logs "$record") images + alert threshold $(get host_alert_threshold_bytes "$record"))"
[ "$(get host_fits "$record")" = yes ] || { echo "FAIL: the host filesystem cannot hold every image" >&2; failed=1; }

echo "== fill test: guest layer"
threshold=$((128 << 20))
vmm "$work/guest" M7_LOG_BYTES=512M "M7_ALERT=$threshold"
load "$work/guest" 2 -interval 0 -max 120s
sleep 1 # one more sample after the load stops
stop
grep '^m7: capacity guest' "$work/guest/console.log" | judge guest "$threshold" ||
	{ echo "FAIL: the guest alert did not fire at its threshold" >&2; failed=1; }

echo "== fill test: host layer"
truncate -s 640M "$work/hostfs.img"
loop=$(losetup -f --show "$work/hostfs.img")
mkfs.ext4 -q "$loop"
mkdir "$work/hostfs"
mount "$loop" "$work/hostfs"
"$repo/scripts/m7/capacity-alert" host "$work/hostfs" "$threshold" 0.5 >"$work/host-alert.log" &
alert=$!
# The volume sits on a test filesystem on purpose, so the host keys cannot match.
vmm "$work/hostfs/vm" M7_LOG_BYTES=1G M7_HOST_CHECK=0
load "$work/hostfs/vm" 3 -interval 0 -max 120s
sleep 1
stop
kill "$alert"
alert=
judge host "$threshold" <"$work/host-alert.log" ||
	{ echo "FAIL: the host alert did not fire at its threshold" >&2; failed=1; }

[ "$failed" = 0 ] || exit 1
echo "pass: VOL-CAPACITY: each layer holds the planned history, and each alert fired at its threshold"
