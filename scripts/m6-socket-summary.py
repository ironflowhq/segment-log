"""Summarizes an m6-socket-report.sh capture: for each mode and side, the median and range of
throughput and latency over the pairs, and the socket's change from in process. Warm-ups are not
counted. A missing or malformed run log fails the summary instead of shrinking it.

    m6-socket-summary.py DIR [SIDE]

SIDE names the second side's logs: unix (the default, M6) or guest (the M7 guest benchmark,
scripts/m7-guest-bench.sh)."""

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


LABELS = {"unix": ("M6 socket diagnostic", "Unix socket"), "guest": ("M7 guest benchmark", "Firecracker guest")}


def summarize(root, side="unix"):
    title, label_side = LABELS[side]
    lines = [f"# {title}", ""]
    for mode in ["closed", "paced"]:
        runs = {s: sorted((root / mode).glob(f"{s}-[0-9]*.log")) for s in ["inproc", side]}
        if not runs["inproc"] or len(runs["inproc"]) != len(runs[side]):
            raise SystemExit(f"{mode}: {len(runs['inproc'])} in-process runs and {len(runs[side])} {side} runs")
        parsed = {s: [parse(p) for p in paths] for s, paths in runs.items()}
        lines += [f"## {mode} ({len(runs['inproc'])} pairs; median, min to max)", "",
                  f"| Metric | In process | {label_side} | Change |", "| --- | --- | --- | --- |"]
        for field, _, label in FIELDS:
            cells, medians = [], []
            for s in ["inproc", side]:
                values = [run[field] for run in parsed[s]]
                medians.append(statistics.median(values))
                cells.append(f"{show(field, medians[-1])} ({show(field, min(values))} to {show(field, max(values))})")
            change = (medians[1] - medians[0]) / medians[0] * 100
            lines.append(f"| {label} | {cells[0]} | {cells[1]} | {change:+.1f}% |")
        lines.append("")
    return "\n".join(lines)


if __name__ == "__main__":
    print(summarize(Path(sys.argv[1]), *sys.argv[2:3]))
