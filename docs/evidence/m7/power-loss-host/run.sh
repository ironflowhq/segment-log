#!/usr/bin/env bash
# The power-loss session: host-power-loss and its control, then the other two host events on this box.
cd "$(dirname "$0")/.."
export M7_HOST=root@195.201.169.204 M7_OBSERVER=178.156.201.194:7070 M7_OUT=$PWD/m7-power-loss-out M7_ROBOT_SERVER=3115035
export M7_ROBOT_CURLRC="<robot curlrc>"
for e in host-power-loss host-kernel-crash host-reset; do
	for c in "" --control; do
		n=hc-$e${c:+-control}; rc=0
		echo "== $n $(date -u +%T)"
		scripts/m7-host-cut.sh "$e" 30 $c >"$M7_OUT/$n.log" 2>&1 || rc=$?
		printf '%s\t%s\t%s\n' "$n" "$rc" "$(date -u +%FT%TZ)" >>"$M7_OUT/results.tsv"
		echo "   exit $rc: $(tail -n 1 "$M7_OUT/$n.log")"
	done
done
ssh -o BatchMode=yes "$M7_HOST" 'for d in nvme0 nvme1; do echo "== $d"; nvme smart-log /dev/$d | grep -E "^(power_cycles|unsafe_shutdowns|power_on_hours)"; done; journalctl --list-boots --no-pager | wc -l' >"$M7_OUT/smart-after.txt"
echo "== done $(date -u +%T)"
