#!/usr/bin/env python3
"""Summarize the five default runs and, when present, the five paced runs of one capture.

No diagnostic run enters a table. The default (closed-loop) runs measure the throughput target.
The paced runs measure the latency target (ADR 0045).
"""
import re
import statistics
import sys
from pathlib import Path


def parse(path):
    text = path.read_text()
    assert "test result: ok. 1 passed" in text, path

    def fields(prefix):
        line = next(line for line in text.splitlines() if line.startswith(prefix))
        return dict(re.findall(r"(\w+)=([\d.]+)", line))

    run = {name: fields(prefix) for name, prefix in [
        ("config", "CONFIG "), ("append", "APPEND "), ("latency", "METRIC append_ns "),
        ("device", "METRIC device_fdatasync_ns "), ("groups", "METRIC group_submits "),
        ("flush", "STORAGE kind=Group.SyncData "), ("reopen", "REOPEN ")]}
    assert all(run["append"][k] == "0" for k in (
        "full", "other_admission_errors", "commit_errors", "correctness_errors")), path
    assert run["reopen"]["correctness_errors"] == "0", path
    if any(line.startswith("PACED ") for line in text.splitlines()):
        run["paced"] = fields("PACED ")
    # Captures from before this line existed have only the device measurement.
    if any(line.startswith("METRIC group_fdatasync_ns ") for line in text.splitlines()):
        run["group_flush"] = fields("METRIC group_fdatasync_ns ")
    return run


def ms(run, group, name):
    return int(run[group][name]) / 1e6


def bound(run):
    return (int(run["config"]["linger_ns"]) + 2 * int(run["device"]["p99"])) / 1e6


def group_bound(run):
    """The bound with the p99 of this run's own group flushes in place of the device measurement."""
    if "group_flush" not in run:
        return None
    return (int(run["config"]["linger_ns"]) + 2 * int(run["group_flush"]["p99"])) / 1e6


def cell(value):
    return "n/a" if value is None else f"{value:.3f}"


def mean(run):
    return int(run["latency"]["sum"]) / int(run["latency"]["count"]) / 1e6


def table(header, rows):
    print("| " + " | ".join(header) + " |")
    print("| " + " | ".join("---" for _ in header) + " |")
    for row in rows:
        print("| " + " | ".join(v if isinstance(v, str) else str(v) if isinstance(v, int) else f"{v:.3f}"
                                for v in row) + " |")


def spread(name, values):
    print(f"\n{name}: median {statistics.median(values):.3f}, range {min(values):.3f}–{max(values):.3f}.")


def main(root):
    runs = [parse(root / f"run-{n}.log") for n in range(1, 6)]
    assert not any("paced" in run for run in runs), "a default run was paced"
    print("# Linux default baseline\n")
    table(["Run", "Records/s", "Append p50 ms", "p99 ms", "p999 ms", "Device p50 ms", "p99 ms",
           "Relative bound ms", "Reopen ms", "Segment bytes"],
          [[n, float(run["append"]["records_per_second"]), ms(run, "latency", "p50"),
            ms(run, "latency", "p99"), ms(run, "latency", "p999"), ms(run, "device", "p50"),
            ms(run, "device", "p99"), bound(run), int(run["reopen"]["ns"]) / 1e6,
            int(run["reopen"]["end_bytes"])] for n, run in enumerate(runs, 1)])
    for name, value in [
            ("Throughput records/s", lambda run: float(run["append"]["records_per_second"])),
            ("Append p50 ms", lambda run: ms(run, "latency", "p50")),
            ("Append p99 ms", lambda run: ms(run, "latency", "p99")),
            ("Append p999 ms", lambda run: ms(run, "latency", "p999")),
            ("Device p50 ms", lambda run: ms(run, "device", "p50")),
            ("Device p99 ms", lambda run: ms(run, "device", "p99")),
            ("Reopen ms", lambda run: int(run["reopen"]["ns"]) / 1e6)]:
        spread(name, [value(run) for run in runs])

    # In a closed loop the mean latency is the caller count divided by the throughput, so this
    # latency follows the throughput and is not the target comparison (ADR 0045).
    print("\n## Closed-loop latency beside callers divided by throughput\n")
    table(["Run", "Mean ms", "Callers / throughput ms", "Groups", "Mean group flush µs"],
          [[n, mean(run),
            1e3 * int(run["config"]["callers"]) / float(run["append"]["records_per_second"]),
            int(run["groups"]["count"]),
            f"{int(run['flush']['sum_ns']) / int(run['flush']['count']) / 1e3:.0f}"]
           for n, run in enumerate(runs, 1)])

    paths = [root / f"paced-{n}.log" for n in range(1, 6)]
    if not paths[0].exists():
        return
    paced = [parse(path) for path in paths]
    assert all("paced" in run for run in paced), "a paced run was not paced"
    offered = float(paced[0]["paced"]["offered_records_per_second"])
    print(f"\n# Linux paced runs at {offered:,.0f} records/s\n")
    table(["Run", "Records/s", "Mean ms", "p50 ms", "p99 ms", "p999 ms", "Max ms", "Device p99 ms",
           "Bound ms", "p99 minus bound ms", "Groups", "Mean group flush µs"],
          [[n, float(run["append"]["records_per_second"]), mean(run), ms(run, "latency", "p50"),
            ms(run, "latency", "p99"), ms(run, "latency", "p999"), ms(run, "latency", "max"),
            ms(run, "device", "p99"), bound(run), f"{ms(run, 'latency', 'p99') - bound(run):+.3f}",
            int(run["groups"]["count"]),
            f"{int(run['flush']['sum_ns']) / int(run['flush']['count']) / 1e3:.0f}"]
           for n, run in enumerate(paced, 1)])
    spread("Paced p50 ms", [ms(run, "latency", "p50") for run in paced])
    spread("Paced p99 ms", [ms(run, "latency", "p99") for run in paced])
    spread("Paced bound ms", [bound(run) for run in paced])
    within = sum(ms(run, "latency", "p99") <= bound(run) for run in paced)
    print(f"\nPaced runs with p99 within the bound of the same run: {within} of {len(paced)}.")

    # The device measurement syncs 256 KiB appends. A paced group is far smaller, so the same
    # formula with the flushes that the run really made is a second, usually tighter, reading.
    print("\n## Paced p99 beside the bound from the run's own group flushes\n")
    table(["Run", "Group flush p50 ms", "Group flush p99 ms", "Bound from group flushes ms",
           "p99 minus that bound ms"],
          [[n, cell(ms(run, "group_flush", "p50") if "group_flush" in run else None),
            cell(ms(run, "group_flush", "p99") if "group_flush" in run else None),
            cell(group_bound(run)),
            "n/a" if group_bound(run) is None else f"{ms(run, 'latency', 'p99') - group_bound(run):+.3f}"]
           for n, run in enumerate(paced, 1)])
    if all("group_flush" in run for run in paced):
        within = sum(ms(run, "latency", "p99") <= group_bound(run) for run in paced)
        print(f"\nPaced runs with p99 within the bound from their own group flushes: {within} of {len(paced)}.")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
