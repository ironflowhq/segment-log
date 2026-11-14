#!/bin/bash
# The M6 footprint check (ADR 0056): the stripped server binary's size, and its resident set on an
# empty log after open, with no connection. The limits are the PRD targets (docs/prd.md, footprint
# and performance targets). Linux only, because it reads /proc.
# Usage: scripts/m6-footprint.sh SERVER_BINARY
set -euo pipefail
binary="${1:?usage: m6-footprint.sh SERVER_BINARY}"
max_bytes=$((4 * 1024 * 1024))
max_rss_kib=$((8 * 1024))

work="$(mktemp -d)"
trap 'kill "$pid" 2>/dev/null || true; rm -rf "$work"' EXIT
cp "$binary" "$work/server"
strip "$work/server"
bytes="$(stat -c %s "$work/server")"

"$work/server" --dir "$work/log" --unix "$work/socket" 2> "$work/stderr" &
pid=$!
for _ in $(seq 100); do
    [ -S "$work/socket" ] && break
    sleep 0.1
done
[ -S "$work/socket" ] || { cat "$work/stderr"; echo "the server did not listen" >&2; exit 1; }
rss_kib="$(awk '/^VmRSS:/ { print $2 }' "/proc/$pid/status")"
kill -TERM "$pid"
wait "$pid"

echo "stripped binary: $bytes bytes (target at most $max_bytes)"
echo "idle resident set: $rss_kib KiB (target at most $max_rss_kib)"
status=0
[ "$bytes" -le "$max_bytes" ] || { echo "binary size misses the target" >&2; status=1; }
[ "$rss_kib" -le "$max_rss_kib" ] || { echo "idle resident set misses the target" >&2; status=1; }
exit "$status"
