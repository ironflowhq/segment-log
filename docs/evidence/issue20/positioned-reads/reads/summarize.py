#!/usr/bin/env python3
"""Summarize retained concurrent_probe output. Buckets are pooled within a run, never across runs."""
import csv
import math
import pathlib
import sys


def bucket(bins, fraction):
    target = math.ceil(sum(count for _, _, count in bins) * fraction)
    seen = 0
    for low, high, count in sorted(bins):
        seen += count
        if seen >= target:
            return f"{low / 1000:.0f}–{high / 1000:.0f}"


root = pathlib.Path(sys.argv[1])
print("| Binary | Mode | Run | Append records/s | Append p50 µs | Append p99 µs | Read calls | Read p50 µs | Read p99 µs | Reads over 100 µs |")
print("| --- | --- | ---: | ---: | --- | --- | ---: | --- | --- | ---: |")
for binary in sys.argv[2:]:
    for mode in ["concurrent", "append"]:
        for path in sorted(root.glob(f"{binary}-{mode}-*.txt")):
            rows = list(csv.reader(path.read_text().splitlines()))
            assert ["success_counts", "append_refusals=0", "commit_errors=0", "read_errors=0", "validation_errors=0"] in rows, path
            assert not any(row[0] == "failure" for row in rows), path
            elapsed, committed, _, watermark = (int(x) for x in next(r for r in rows if r[0] == "measurement")[1:])
            assert committed == 32768 and watermark == 33792, path
            bins = {}
            for row in rows:
                if row[0] == "bin":
                    bins.setdefault(row[2], []).append(tuple(int(x) for x in row[3:]))
            reads = bins.get("start", []) + bins.get("end", [])
            calls = sum(count for _, _, count in reads)
            slow = sum(count for low, _, count in reads if low >= 100_000)
            cells = [f"{calls:,}", bucket(reads, 0.5), bucket(reads, 0.99), f"{slow:,}"] if calls else ["–"] * 4
            print(f"| {binary} | {mode} | {path.stem.rsplit('-', 1)[1]} | {committed / (elapsed / 1e9):,.0f} | "
                  f"{bucket(bins['append'], 0.5)} | {bucket(bins['append'], 0.99)} | " + " | ".join(cells) + " |")
