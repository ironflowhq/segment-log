#!/usr/bin/env bash
# Issue 177: random and scan reads of sealed logs through LogReader, before and after ADR 0069's
# sparse frame tables, on the same logs. Raw output goes to OUT_DIR, outside docs/evidence.
#
#   docs/evidence/issue177/run.sh OUT_DIR [REPS] [SECONDS] [BEFORE]
#
# Derived from the issue 156 driver. BEFORE (default 0a11998, the commit before the change) and
# HEAD each build readers_probe from HEAD's source: the probe uses only the public API, which the
# change leaves alone, so only the library differs. Each case's log is built once with HEAD's
# probe and sealed. Then, per case:
#
# - frames per segment and the log's bytes against RAM (logstat.py);
# - one discarded `sealed` pass per commit, which walks every segment before any timed run;
# - REPS rounds, each running random and scan at both commits in an order shuffled with the round
#   as seed: one reader, 64 records per call, for SECONDS. Each run first reads one record at each
#   segment's base and reports those first walks on a WALK line, apart from its READ line;
# - the fraction of the log resident in the page cache after the timed runs.
#
# Cases: the issue 156 logs (1,500,000, 4,000,000 and 10,000,000 records: about 7, 18 and 43
# segments); about 200 segments at the default roll; 1,000 single-record callers, run until the
# log holds CALLER_SEGMENTS segments; one caller with one record per group, SINGLE records at a
# 4 MiB roll (about 20 segments, about 30 minutes to build on an M4).
#
# Sizes are environment variables so that a smoke run can shrink them. CALLER_SECONDS (default 60)
# is the length of each 1,000-caller build round, so a smaller log can stop nearer its target.
set -euo pipefail
out=${1:?usage: run.sh OUT_DIR [REPS] [SECONDS] [BEFORE]}
reps=${2:-3}
seconds=${3:-10}
before=${4:-0a11998}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)
case "$out/" in "$root"/docs/evidence/*) echo "OUT_DIR must be outside docs/evidence" >&2; exit 1 ;; esac
cases_sizes=${CASES:-"s7:1500000 s18:4000000 s43:10000000 s200:46500000 callers:${CALLER_SEGMENTS:-300} single:${SINGLE:-254000}"}

{
	echo "date: $(date -u +%FT%TZ)"
	echo "commit: $(git -C "$root" rev-parse HEAD)"
	echo "before: $(git -C "$root" rev-parse "$before")"
	git -C "$root" status --porcelain | sed 's/^/dirty: /'
	uname -a
	if [ "$(uname)" = Darwin ]; then
		sysctl -n machdep.cpu.brand_string hw.ncpu hw.memsize
		diskutil info "$(df "$out" | awk 'NR==2 { print $1 }')" | grep -E 'Device / Media Name|File System Personality|Solid State|Protocol' || true
	else
		lscpu | grep -E 'Model name|^CPU\(s\)|Thread|NUMA node\(s\)'
		free -b | head -2
		findmnt -T "$out"
		source=$(findmnt -n -o SOURCE -T "$out")
		# A filesystem on a whole disk has no parent, so fall back to the source itself.
		disk=$(lsblk -no PKNAME "$source" 2>/dev/null | head -1)
		[ -n "$disk" ] && source=/dev/$disk
		lsblk -d -o NAME,MODEL,REV,SIZE "$source" || true
	fi
	rustc --version
} >"$out/environment.txt"
cat "$out/environment.txt"

# Both binaries come from HEAD's probe source; only the library under it differs.
cd "$root"
cargo build -q --release -p segment-log-core --example readers_probe
mkdir -p "$out/bin"
cp target/release/examples/readers_probe "$out/bin/head"
rm -rf "$out/before-src"
git worktree add -q --detach "$out/before-src" "$before"
cp segment-log-core/examples/readers_probe.rs "$out/before-src/segment-log-core/examples/readers_probe.rs"
(cd "$out/before-src" && CARGO_TARGET_DIR="$out/before-target" cargo build -q --release -p segment-log-core --example readers_probe)
cp "$out/before-target/release/examples/readers_probe" "$out/bin/before"
git worktree remove --force "$out/before-src"
probe() { "$out/bin/$1" "${@:2}"; }

segment_count() { ls "$1" | grep -c '\.seg$'; }

# build NAME SIZE LOG
build() {
	case $1 in
	callers)
		probe head build "$3" 1000 1000 >/dev/null
		while [ "$(segment_count "$3")" -lt "$2" ]; do
			probe head run "$3" 1000 0 tail "${CALLER_SECONDS:-60}" 64 | grep APPEND
		done
		;;
	single) probe head build-single "$3" "$2" $((4 * 1024 * 1024)) ;;
	*) probe head build "$3" "$2" 1000 ;;
	esac
	probe head seal "$3"
}

: >"$out/reads.txt"
for c in $cases_sizes; do
	name=${c%%:*}
	log=$out/log-$name
	rm -rf "$log"
	echo "== build $name" | tee -a "$out/reads.txt"
	build "$name" "${c#*:}" "$log" | tee -a "$out/reads.txt"
	echo "CASE name=$name segments=$(segment_count "$log") $(python3 -I "$here/logstat.py" frames "$log")" | tee -a "$out/reads.txt"
	for commit in before head; do
		probe "$commit" sealed "$log" 1 scan 2 64 >"$out/warmup-$name-$commit.txt"
	done
	for rep in $(seq "$reps"); do
		order=$(python3 -I -c 'import random, sys; c = sys.argv[2:]; random.Random(int(sys.argv[1])).shuffle(c); print(*c)' "$rep" before:random before:scan head:random head:scan)
		for r in $order; do
			echo "RUN name=$name commit=${r%%:*} rep=$rep pattern=${r#*:}" | tee -a "$out/reads.txt"
			probe "${r%%:*}" sealed "$log" 1 "${r#*:}" "$seconds" 64 | tee -a "$out/reads.txt"
		done
	done
	echo "AFTER name=$name $(python3 -I "$here/logstat.py" resident "$log")" | tee -a "$out/reads.txt"
	rm -rf "$log"
done

python3 -I "$here/summarize.py" "$out/reads.txt" | tee "$out/summary.txt"
