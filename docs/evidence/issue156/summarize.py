"""Medians of a run.sh output directory (issue 156), as Markdown tables.

    summarize.py OUT_DIR
"""
import collections
import statistics
import sys

out = sys.argv[1]


def fields(line):
    return dict(f.split("=", 1) for f in line.split()[1:] if "=" in f)


def med(rows, key):
    values = [float(r[key]) for r in rows if r.get(key, "-") != "-"]
    return f"{statistics.median(values):,.0f}" if values else "-"


def span(rows, key):
    values = [float(r[key]) for r in rows if r.get(key, "-") != "-"]
    return f"{min(values):,.0f}–{max(values):,.0f}" if values else "-"


appends, reads = collections.defaultdict(list), collections.defaultdict(list)
for line in open(f"{out}/readers.txt"):
    if line.startswith("APPEND"):
        f = fields(line)
        appends[(int(f["readers"]), f["pattern"])].append(f)
    elif line.startswith("READ"):
        f = fields(line)
        if f["readers"] != "0":
            reads[(f["kind"], int(f["readers"]), f["pattern"])].append(f)

print("## Appends beside readers (1,000 appenders)\n")
print("| Readers | Pattern | Runs | Records/s median (range) | p50 µs | p99 µs | p999 µs | Full |")
print("| ---: | --- | ---: | --- | ---: | ---: | ---: | ---: |")
for (r, p), rows in sorted(appends.items()):
    print(f"| {r} | {p if r else '-'} | {len(rows)} | {med(rows, 'records_per_s')} ({span(rows, 'records_per_s')}) | "
          f"{med(rows, 'p50_us')} | {med(rows, 'p99_us')} | {med(rows, 'p999_us')} | {med(rows, 'full')} |")

print("\n## Reads (64 records per call)\n")
print("| Log | Readers | Pattern | Runs | Records/s median (range) | p50 µs | p99 µs | p999 µs | max µs |")
print("| --- | ---: | --- | ---: | --- | ---: | ---: | ---: | ---: |")
for (k, r, p), rows in sorted(reads.items()):
    print(f"| {k} | {r} | {p} | {len(rows)} | {med(rows, 'records_per_s')} ({span(rows, 'records_per_s')}) | "
          f"{med(rows, 'p50_us')} | {med(rows, 'p99_us')} | {med(rows, 'p999_us')} | {med(rows, 'max_us')} |")

segments = collections.defaultdict(list)
for line in open(f"{out}/segments.txt"):
    f = fields(line)
    segments[(int(f["history"]), int(f["segments"]), f["pattern"])].append(f)
print("\n## Frame tables: one reader on a sealed log (LogReader, 64 records per call)\n")
print("| Records | Segments | Pattern | Runs | Records/s median (range) | p50 µs | p99 µs | p999 µs |")
print("| ---: | ---: | --- | ---: | --- | ---: | ---: | ---: |")
for (n, segs, p), rows in sorted(segments.items()):
    print(f"| {n:,} | {segs} | {p} | {len(rows)} | {med(rows, 'records_per_s')} ({span(rows, 'records_per_s')}) | "
          f"{med(rows, 'p50_us')} | {med(rows, 'p99_us')} | {med(rows, 'p999_us')} |")

rss = collections.defaultdict(list)
for line in list(open(f"{out}/streams.tsv"))[1:]:
    rep, streams, mode, barrier, kib = line.split()
    rss[(int(streams), mode, barrier)].append(int(kib))
print("\n## Resident set by stream count (1,000,000 records, no keys), median KiB\n")
print("| Streams | After open | After appends | After recovery | Appended − open | Recovered − baseline |")
print("| ---: | ---: | ---: | ---: | ---: | ---: |")
m = lambda k: statistics.median(rss[k]) if rss.get(k) else float("nan")
for s in sorted({k[0] for k in rss}):
    opened, appended = m((s, "append", "empty_held")), m((s, "append", "appended_held"))
    base, recovered = m((s, "recover", "baseline")), m((s, "recover", "recovered_held"))
    print(f"| {s:,} | {opened:,.0f} | {appended:,.0f} | {recovered:,.0f} | {appended - opened:,.0f} | {recovered - base:,.0f} |")
