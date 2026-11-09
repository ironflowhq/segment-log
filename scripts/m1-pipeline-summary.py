#!/usr/bin/env python3
"""Join request and group clocks, reject invalid traces, and attribute slow requests."""
import collections
import csv
import json
import math
import pathlib
import re
import statistics
import sys


def rows(path):
    with open(path) as source:
        return [{key: int(value) for key, value in row.items()}
                for row in csv.DictReader(source, delimiter="\t")]


def summarize(path):
    requests = rows(path)
    group_rows = rows(path.with_suffix(".groups.tsv"))
    groups = {row["group"]: row for row in group_rows}
    assert requests and groups and len(groups) == len(group_rows)
    identities = set()
    offsets = set()
    counts = collections.Counter()
    stages = ["start", "admission", "enqueue", "dequeue", "classify", "ready",
              "formed", "encoded", "written", "flushed", "published", "send", "wake"]
    intervals = ["input_and_retries", "admission_before_send", "queue_and_send",
                 "writer_before_classify", "classification", "remaining_formation",
                 "encode", "write", "flush", "publication", "delivery_prefix", "send_to_wake"]
    timelines = []
    for request in requests:
        identity = (request["caller"], request["revision"])
        assert identity not in identities, ("duplicate", identity)
        identities.add(identity)
        assert request["offset_plus_one"] not in offsets
        offsets.add(request["offset_plus_one"])
        group = groups[request["group"]]
        assert group["group"] <= request["offset_plus_one"] < group["group"] + group["count"]
        counts[request["group"]] += 1
        values = {**group, **request}
        times = [values[key] for key in stages]
        assert all(t > 0 for t in times), ("missing clock", identity)
        assert times == sorted(times), ("nonmonotonic", identity, values)
        assert request["enqueue"] <= request["admitted"] <= request["wake"]
        assert group["published"] <= group["delivered"]
        assert request["send"] <= group["delivered"]
        pieces = [b - a for a, b in zip(times, times[1:])]
        total = times[-1] - times[0]
        assert sum(pieces) == total
        timelines.append((total, pieces, identity))
    assert counts == {key: group["count"] for key, group in groups.items()}
    assert offsets == set(range(1, len(requests) + 1))
    callers = max(caller for caller, _ in identities) + 1
    rounds = max(revision for _, revision in identities) + 1
    assert identities == {(caller, revision) for caller in range(callers) for revision in range(rounds)}
    timelines.sort()
    def percentile(fraction):
        return timelines[math.ceil(len(timelines) * fraction) - 1][0]
    tail = timelines[-max(1, math.ceil(len(timelines) / 100)):]
    tail_total = sum(row[0] for row in tail)
    return dict(requests=len(requests), groups=len(groups), invariant_errors=0,
                p50_ns=percentile(.5), p99_ns=percentile(.99), p999_ns=percentile(.999),
                tail_count=len(tail), tail_total_ns=tail_total,
                tail_stage_ns={name: sum(row[1][i] for row in tail) for i, name in enumerate(intervals)},
                tail_stage_fraction={name: sum(row[1][i] for row in tail) / tail_total
                                     for i, name in enumerate(intervals)},
                p99_request=dict(identity=timelines[math.ceil(len(timelines) * .99)-1][2],
                                 stages_ns=dict(zip(intervals, timelines[math.ceil(len(timelines)*.99)-1][1]))))


def comparison(root):
    result = {}
    for mode in ["control", "trace"]:
        runs = []
        for repetition in range(1, 6):
            directory = root / f"{repetition}-{mode}"
            log = (directory / "run.log").read_text()
            manifest = json.loads((directory / "manifest.json").read_text())
            assert manifest["exit"] == 0
            assert "other_admission_errors=0 commit_errors=0 correctness_errors=0" in log
            assert "test result: ok. 1 passed" in log and "REOPEN " in log
            match = re.search(r"METRIC append_ns .*p50=(\d+) p99=(\d+) p999=(\d+)", log)
            runs.append(dict(run=directory.name, p50_ms=int(match[1])/1e6,
                             p99_ms=int(match[2])/1e6, p999_ms=int(match[3])/1e6,
                             rps=float(re.search(r"records_per_second=([\d.]+)", log)[1])))
        result[mode] = dict(runs=runs, summary={
            key: dict(median=statistics.median([run[key] for run in runs]),
                      min=min(run[key] for run in runs), max=max(run[key] for run in runs))
            for key in ["p50_ms", "p99_ms", "p999_ms", "rps"]})
    return result


if __name__ == "__main__":
    path = pathlib.Path(sys.argv[1])
    print(json.dumps(comparison(path) if path.is_dir() else summarize(path), indent=2))
