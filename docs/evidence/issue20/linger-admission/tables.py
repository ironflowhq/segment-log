#!/usr/bin/env python3
"""Tabulate one `m1-compare` output directory: usage tables.py DIRECTORY.

Prints the paced pairs, the closed-loop pairs and the traced repetitions as Markdown tables.
"""
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])


def load(path):
    text = path.read_text()
    assert "test result: ok. 1 passed; 0 failed" in text, path
    assert "full=0 other_admission_errors=0 commit_errors=0 correctness_errors=0" in text, path

    def fields(marker):
        line = next(line for line in text.splitlines() if line.startswith(marker))
        return dict(re.findall(r"(\w+)=([0-9.]+)", line))

    return {name: fields(marker) for name, marker in [
        ("config", "CONFIG "), ("append", "APPEND "), ("latency", "METRIC append_ns "),
        ("device", "METRIC device_fdatasync_ns "), ("groups", "METRIC group_submits "),
        ("flush", "METRIC group_fdatasync_ns ")]}


def ms(run, group, name):
    return int(run[group][name]) / 1e6


def bound(run, source):
    return (int(run["config"]["linger_ns"]) + 2 * int(run[source]["p99"])) / 1e6


def table(header, rows):
    print("| " + " | ".join(header) + " |")
    print("| --- |" + " ---: |" * (len(header) - 1))
    for row in rows:
        print("| " + " | ".join(row) + " |")
    print()


def runs(mode):
    return [(f"{side} {n}", load(root / mode / f"{side}-{n}.log"))
            for side in ("before", "after") for n in range(1, 6)]


print("Paced at 50,000 records/s\n")
table(["Run", "Mean ms", "p50 ms", "p99 ms", "p999 ms", "Max ms", "Bound ms", "Bound from group flushes ms",
       "Groups", "Group flush p50 ms", "p99 ms", "Max ms"],
      [[name, f"{int(run['latency']['sum']) / int(run['latency']['count']) / 1e6:.3f}",
        f"{ms(run, 'latency', 'p50'):.3f}", f"{ms(run, 'latency', 'p99'):.3f}",
        f"{ms(run, 'latency', 'p999'):.3f}", f"{ms(run, 'latency', 'max'):.3f}",
        f"{bound(run, 'device'):.3f}", f"{bound(run, 'flush'):.3f}", f"{int(run['groups']['count']):,}",
        f"{ms(run, 'flush', 'p50'):.3f}", f"{ms(run, 'flush', 'p99'):.3f}", f"{ms(run, 'flush', 'max'):.3f}"]
       for name, run in runs("paced")])
print("Closed loop\n")
table(["Run", "Records/s", "p50 ms", "p99 ms", "p999 ms", "Groups", "Records for each group",
       "Group flush p50 ms", "p99 ms"],
      [[name, f"{float(run['append']['records_per_second']):,.0f}", f"{ms(run, 'latency', 'p50'):.3f}",
        f"{ms(run, 'latency', 'p99'):.3f}", f"{ms(run, 'latency', 'p999'):.3f}",
        f"{int(run['groups']['count']):,}", f"{int(run['append']['records']) / int(run['groups']['count']):.0f}",
        f"{ms(run, 'flush', 'p50'):.3f}", f"{ms(run, 'flush', 'p99'):.3f}"]
       for name, run in runs("latency")])
print("Traced paced repetitions: admission to the end of gathering, and the whole request\n")
rows = []
for side in ("before", "after"):
    for n in (1, 2, 3):
        text = (root / "trace" / f"{side}-{n}.phases.txt").read_text()
        gather = re.search(r"gather_wait\s+(.*)", text).group(1).split()
        total = next(line for line in text.splitlines() if line.strip().startswith("total")).split()
        rows.append([f"{side} {n}", gather[0], gather[1], gather[2], gather[3], total[2], total[3]])
table(["Repetition", "Gather mean ms", "p50 ms", "p90 ms", "p99 ms", "Request p50 ms", "p99 ms"], rows)
