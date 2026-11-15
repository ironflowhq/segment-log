"""Summarizes an m6-socket-report.sh capture: for each mode and side, the median and range of
throughput and latency over the pairs, and the socket's change from in process. Warm-ups are not
counted. A missing or malformed run log fails the summary instead of shrinking it."""

import re
import statistics
import sys
from pathlib import Path

FIELDS = [("records_per_second", "APPEND", "records/s"), ("p50", "METRIC append_ns", "p50 ms"),
          ("p99", "METRIC append_ns", "p99 ms"), ("p999", "METRIC append_ns", "p999 ms")]


def parse(path):
    values = {}
    for line in path.read_text().splitlines():
        for field, prefix, _ in FIELDS:
            if line.startswith(prefix + " "):
                match = re.search(rf"\b{field}=([0-9.]+)", line)
                if match:
                    values[field] = float(match.group(1))
    missing = [f for f, _, _ in FIELDS if f not in values]
    if missing:
        raise SystemExit(f"{path}: missing {', '.join(missing)}")
    return values


def show(field, value):
    return f"{value:,.0f}" if field == "records_per_second" else f"{value / 1e6:.3f}"


def summarize(root):
    lines = ["# M6 socket diagnostic", ""]
    for mode in ["closed", "paced"]:
        runs = {side: sorted((root / mode).glob(f"{side}-[0-9]*.log")) for side in ["inproc", "unix"]}
        if not runs["inproc"] or len(runs["inproc"]) != len(runs["unix"]):
            raise SystemExit(f"{mode}: {len(runs['inproc'])} in-process runs and {len(runs['unix'])} socket runs")
        parsed = {side: [parse(p) for p in paths] for side, paths in runs.items()}
        lines += [f"## {mode} ({len(runs['inproc'])} pairs; median, min to max)", "",
                  "| Metric | In process | Unix socket | Socket change |", "| --- | --- | --- | --- |"]
        for field, _, label in FIELDS:
            cells, medians = [], []
            for side in ["inproc", "unix"]:
                values = [run[field] for run in parsed[side]]
                medians.append(statistics.median(values))
                cells.append(f"{show(field, medians[-1])} ({show(field, min(values))} to {show(field, max(values))})")
            change = (medians[1] - medians[0]) / medians[0] * 100
            lines.append(f"| {label} | {cells[0]} | {cells[1]} | {change:+.1f}% |")
        lines.append("")
    return "\n".join(lines)


if __name__ == "__main__":
    print(summarize(Path(sys.argv[1])))
