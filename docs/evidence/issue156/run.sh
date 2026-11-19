#!/usr/bin/env bash
# Issue 156: several readers beside several appenders on a large history, and resident memory
# per stream. Raw output goes to OUT_DIR, outside docs/evidence.
#
#   docs/evidence/issue156/run.sh OUT_DIR [RECORDS] [SECONDS] [REPS]
#
# Readers: readers_probe builds RECORDS keyed 256-byte records over 1,000 streams once. Every case
# then runs on its own copy-on-write clone of that history, so each one starts from the same log:
# 1,000 closed-loop appenders for SECONDS beside 0, 1, 4 or 16 readers that read 64 records per
# call (tail, random or scan). Each of REPS rounds runs every case once, in an order shuffled with
# the round number as seed. Then a sealed clone is read through LogReader with no appender.
#
# Frame tables: sealed logs of 1,500,000, 4,000,000 and 10,000,000 records (about 7, 18 and 43
# segments, around the 16 frame tables a log keeps), each read at random and in order by one
# reader through LogReader.
#
# Memory: streams.py runs resident_probe (append, then recover) on 1,000,000 records over 1 to
# 1,000,000 streams, and samples the resident set at its barriers.
set -euo pipefail
out=${1:?usage: run.sh OUT_DIR [RECORDS] [SECONDS] [REPS]}
records=${2:-10000000}
seconds=${3:-20}
reps=${4:-3}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)
case "$out/" in "$root"/docs/evidence/*) echo "OUT_DIR must be outside docs/evidence" >&2; exit 1 ;; esac

# A clone shares the history's blocks until the clone writes, so a case costs no full copy.
if [ "$(uname)" = Darwin ]; then
	clone() { cp -cR "$1" "$2"; }
else
	clone() { cp -R --reflink=auto "$1" "$2"; }
fi

{
	echo "date: $(date -u +%FT%TZ)"
	echo "commit: $(git -C "$root" rev-parse HEAD)"
	# Untracked files count: a probe that is not committed yet is part of what ran.
	git -C "$root" status --porcelain | sed 's/^/dirty: /'
	uname -a
	if [ "$(uname)" = Darwin ]; then
		sysctl -n machdep.cpu.brand_string hw.ncpu hw.memsize
		diskutil info "$(df "$out" | awk 'NR==2 { print $1 }')" | grep -E 'Device / Media Name|File System Personality|Solid State|Protocol' || true
	else
		lscpu | grep -E 'Model name|^CPU\(s\)|Thread|NUMA node\(s\)'
		free -b | head -2
		findmnt -T "$out"
		lsblk -d -o NAME,MODEL,REV,SIZE "$(findmnt -n -o SOURCE -T "$out" | sed 's/p\?[0-9]*$//')" || true
	fi
	rustc --version
} >"$out/environment.txt"
cat "$out/environment.txt"

cd "$root"
cargo build -q --release -p segment-log-core --example readers_probe --example resident_probe
bin=$root/target/release/examples

base=$out/history
rm -rf "$base" "$out/case" "$out/sealed"
"$bin/readers_probe" build "$base" "$records" 1000 | tee "$out/build.txt"
du -sk "$base" >"$out/log-size.txt"
ls "$base" | wc -l >>"$out/log-size.txt"

# run_case READERS PATTERN: one case on a fresh clone of the history.
run_case() {
	clone "$base" "$out/case"
	"$bin/readers_probe" run "$out/case" 1000 "$1" "$2" "$seconds" 64
	rm -rf "$out/case"
}

# One discarded case first, so that no measured case pays for a cold page cache or a cold build.
run_case 0 tail >"$out/warmup.txt"
cases="0:tail 1:tail 1:random 1:scan 4:tail 4:random 4:scan 16:tail 16:random 16:scan"
: >"$out/readers.txt"
for rep in $(seq "$reps"); do
	order=$(python3 -c 'import random, sys; c = sys.argv[2:]; random.Random(int(sys.argv[1])).shuffle(c); print(*c)' "$rep" $cases)
	echo "ORDER rep=$rep $order" | tee -a "$out/readers.txt"
	for c in $order; do
		echo "== rep $rep readers ${c%%:*} ${c#*:}"
		run_case "${c%%:*}" "${c#*:}" | tee -a "$out/readers.txt"
	done
done

clone "$base" "$out/sealed"
"$bin/readers_probe" seal "$out/sealed" | tee -a "$out/readers.txt"
for rep in $(seq "$reps"); do
	order=$(python3 -c 'import random, sys; c = sys.argv[2:]; random.Random(int(sys.argv[1])).shuffle(c); print(*c)' "$rep" 1:random 1:scan 4:random 4:scan 16:random 16:scan)
	for c in $order; do
		echo "== sealed rep $rep readers ${c%%:*} ${c#*:}"
		"$bin/readers_probe" sealed "$out/sealed" "${c%%:*}" "${c#*:}" "$seconds" 64 | tee -a "$out/readers.txt"
	done
done
rm -rf "$out/sealed" "$base"

: >"$out/segments.txt"
for n in 1500000 4000000 10000000; do
	log=$out/segments-$n
	rm -rf "$log"
	"$bin/readers_probe" build "$log" "$n" 1000 >/dev/null
	"$bin/readers_probe" seal "$log" >/dev/null
	segments=$(ls "$log" | grep -c '\.seg$')
	for rep in $(seq "$reps"); do
		for p in random scan; do
			echo "SEGMENTS history=$n segments=$segments rep=$rep $("$bin/readers_probe" sealed "$log" 1 "$p" 10 64)" | tee -a "$out/segments.txt"
		done
	done
	rm -rf "$log"
done

python3 "$here/streams.py" "$bin/resident_probe" "$out" "$reps"
python3 "$here/summarize.py" "$out"
