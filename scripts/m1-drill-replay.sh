#!/bin/bash
# Checks the failure artifacts of a long-run drill: usage scripts/m1-drill-replay.sh DIRECTORY.
# DIRECTORY holds what the workflow uploaded, or what `gh run download` fetched.
# Each plain replay must pass, and each replay with the planted record must fail on that record.
set -euo pipefail
dir="$(cd "${1:?usage: scripts/m1-drill-replay.sh DIRECTORY}" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

trace="$(find "$dir" -maxdepth 1 -name '*.trace' | sort | head -n 1)"
test -n "$trace" || { echo "no trace in $dir"; exit 1; }
pair="${trace%.trace}"
test -s "$pair.trace" && test -s "$pair.ledger" || { echo "incomplete pair $pair"; exit 1; }
root="$(find "$dir/process" -maxdepth 1 -type d -name 'segment-log-slice8-*' | sort | head -n 1)"
test -n "$root" || { echo "no process directory in $dir/process"; exit 1; }
test -s "$root/protocol.txt" && test -s "$root/log/00000000000000000000.seg" \
    || { echo "incomplete process directory $root"; exit 1; }
echo "pair=$pair"
echo "process=$root"

trace_test=(cargo test -p segment-log-core --lib acceptance::seeded::saved_trace_replays_the_oracle -- --exact --nocapture)
process_test=(cargo test -p segment-log-core --test process_crash killed_writers_preserve_the_recorded_contract -- --exact --nocapture)
rejects() { # rejects MESSAGE LOG COMMAND... : the command fails and prints the message
    local message="$1" log="$2"
    shift 2
    if "$@" > "$log" 2>&1; then
        echo "the planted record passed: $log"
        exit 1
    fi
    grep -F -- "$message" "$log" || { cat "$log"; exit 1; }
}

SEGMENT_LOG_TRACE_REPLAY="$pair" SEGMENT_LOG_TRACE_DRILL=0 "${trace_test[@]}"
SEGMENT_LOG_TRACE_REPLAY="$pair" SEGMENT_LOG_TRACE_DRILL=1 \
    rejects "false permanent size refusal" "$work/trace.log" "${trace_test[@]}"
# The process oracle appends to the log that it checks, so each replay gets its own copy.
cp -R "$root" "$work/drill"
cp -R "$root" "$work/plain"
SEGMENT_LOG_PROCESS_REPLAY="$work/drill" SEGMENT_LOG_PROCESS_DRILL=1 \
    rejects "log.watermark() >= ledger.watermark" "$work/process.log" "${process_test[@]}"
SEGMENT_LOG_PROCESS_REPLAY="$work/plain" SEGMENT_LOG_PROCESS_DRILL=0 "${process_test[@]}"
echo "drill replay: both plain replays passed and both planted records were rejected"
