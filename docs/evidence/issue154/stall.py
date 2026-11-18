#!/usr/bin/env python3
"""Roll stalls in one traced paced run: usage stall.py DIR.

DIR holds the pipeline trace of `measured_appends_survive_reopen` (SEGMENT_LOG_PIPELINE_OUT=DIR/t.tsv):
t.tsv, t.groups.tsv and t.events.tsv. For each roll and closing run it prints how long the writer
spent, the longest gap between group writes around it, and how long the backlog took to drain.
Then it counts how many of the slowest 1% of requests started inside a closing run or its drain.
"""
import csv
import sys


def rows(path):
    return list(csv.DictReader(open(path), delimiter="\t"))


def main(d):
    events = rows(f"{d}/t.events.tsv")
    groups = [{k: int(v) for k, v in r.items()} for r in rows(f"{d}/t.groups.tsv")]
    latency = sorted((int(r["wake"]) - int(r["start"]), int(r["start"])) for r in rows(f"{d}/t.tsv"))
    n = len(latency)
    print(f"requests {n}  p50 {latency[n // 2][0] / 1e6:.2f} ms  p99 {latency[int(n * .99)][0] / 1e6:.2f} ms")
    slow = [start for _, start in latency[int(n * .99):]]
    median = sorted(g["count"] for g in groups)[len(groups) // 2]
    windows = []
    for e in events:
        name, start, middle, end = e["event"], int(e["start"]), int(e["middle"]), int(e["end"])
        near = [g for g in groups if start - 20e6 < g["written"] < end + 400e6]
        gap = max(((b["written"] - a["written"], b["count"]) for a, b in zip(near, near[1:])), default=(0, 0))
        line = (f"{name:8} {(end - start) / 1e6:6.1f} ms (to middle {(middle - start) / 1e6:5.1f}, "
                f"after {(end - middle) / 1e6:5.1f}); longest gap between group writes "
                f"{gap[0] / 1e6:5.1f} ms, then a group of {gap[1]}")
        if name == "pending":
            # The backlog has drained at the first group after the closing run that is near normal size.
            drained = next((g["formed"] for g in groups if g["formed"] > end and g["count"] < 1.5 * median), end)
            windows.append((start - 40e6, drained))
            line += f"; drained {(drained - end) / 1e6:.1f} ms after"
        print(line)
    inside = sum(1 for s in slow if any(a <= s <= b for a, b in windows))
    print(f"median group {median} submits; slowest 1% = {len(slow)} requests, "
          f"{inside} ({100 * inside / max(len(slow), 1):.0f}%) started in a closing run or its drain")


if __name__ == "__main__":
    main(sys.argv[1])
