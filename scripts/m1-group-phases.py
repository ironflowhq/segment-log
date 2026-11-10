#!/usr/bin/env python3
"""Split the writer's group cycle into phases from one SEGMENT_LOG_PIPELINE_OUT trace.

Reads REQUESTS.tsv and REQUESTS.groups.tsv. A group counts toward the cycle
medians only when it holds at least --min-count requests, so that the warm-up
and the last partial groups do not dilute the steady state.
"""
import argparse
import csv
import pathlib
import statistics

GROUP_PHASES = ["formation", "encode", "write", "flush", "publish", "delivery"]
REQUEST_PHASES = ["admission", "queue", "formation_wait", "encode", "write", "flush", "publish",
                  "answer_wait", "answer_to_wake"]
# The wait from a request's admission until its group stops gathering: queue plus formation_wait.
# A group's linger bounds this wait for the group's first request (ADR 0046).
GATHER = "gather_wait"


def rows(path):
    with open(path) as source:
        return [{key: int(value) for key, value in row.items()}
                for row in csv.DictReader(source, delimiter="\t")]


def quantile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * fraction))]


def summarize(path, min_count):
    """Returns per-group phase samples and per-request interval samples, in nanoseconds."""
    groups = rows(path.with_suffix(".groups.tsv"))
    by_id = {group["group"]: group for group in groups}
    first_dequeue = {}
    requests = {name: [] for name in REQUEST_PHASES + [GATHER, "total"]}
    for request in rows(path):
        group = by_id[request["group"]]
        first_dequeue[request["group"]] = min(first_dequeue.get(request["group"], request["dequeue"]),
                                              request["dequeue"])
        requests["admission"].append(request["enqueue"] - request["start"])
        requests["queue"].append(request["dequeue"] - request["enqueue"])
        requests["formation_wait"].append(group["formed"] - request["dequeue"])
        requests[GATHER].append(group["formed"] - request["enqueue"])
        requests["encode"].append(group["encoded"] - group["formed"])
        requests["write"].append(group["written"] - group["encoded"])
        requests["flush"].append(group["flushed"] - group["written"])
        requests["publish"].append(group["published"] - group["flushed"])
        requests["answer_wait"].append(request["send"] - group["published"])
        requests["answer_to_wake"].append(request["wake"] - request["send"])
        requests["total"].append(request["wake"] - request["start"])
    phases = {name: [] for name in GROUP_PHASES + ["cycle"]}
    previous = None
    for group in groups:
        if group["count"] < min_count:
            previous = None
            continue
        marks = [first_dequeue[group["group"]], group["formed"], group["encoded"], group["written"],
                 group["flushed"], group["published"], group["delivered"]]
        assert marks == sorted(marks), group
        for name, start, end in zip(GROUP_PHASES, marks, marks[1:]):
            phases[name].append(end - start)
        if previous is not None:
            phases["cycle"].append(group["delivered"] - previous["delivered"])
        previous = group
    return len(groups), phases, requests


def slowest(requests, fraction):
    """Returns the latency at `fraction` and, for the requests at or above it, each interval's mean.

    A percentile says how slow the tail is. This says where those requests spent the time.
    """
    totals = requests["total"]
    threshold = quantile(totals, fraction)
    chosen = [index for index, total in enumerate(totals) if total >= threshold]
    means = {name: sum(requests[name][index] for index in chosen) / len(chosen)
             for name in REQUEST_PHASES + ["total"]}
    return threshold, len(chosen), means


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("requests", type=pathlib.Path)
    parser.add_argument("--min-count", type=int, default=1)
    args = parser.parse_args()
    total, phases, requests = summarize(args.requests, args.min_count)
    print(f"groups={total} counted_groups={len(phases['formation'])} min_count={args.min_count}")
    cycle = statistics.median(phases["cycle"])
    print("group phase, ms: median p10 p90 share_of_cycle")
    for name in GROUP_PHASES + ["cycle"]:
        values = phases[name]
        median = statistics.median(values)
        print(f"  {name:<10} {median / 1e6:8.3f} {quantile(values, 0.1) / 1e6:8.3f} "
              f"{quantile(values, 0.9) / 1e6:8.3f} {100 * median / cycle:6.1f}%")
    print("request interval, ms: mean p50 p99 share_of_latency")
    latency = sum(requests["total"])
    for name in REQUEST_PHASES + ["total"]:
        values = requests[name]
        print(f"  {name:<14} {sum(values) / len(values) / 1e6:8.3f} {quantile(values, 0.5) / 1e6:8.3f} "
              f"{quantile(values, 0.99) / 1e6:8.3f} {100 * sum(values) / latency:6.1f}%")
    for label, fraction in [("1%", 0.99), ("0.1%", 0.999)]:
        threshold, count, means = slowest(requests, fraction)
        print(f"slowest {label} of requests ({count} at or above {threshold / 1e6:.3f} ms), "
              "ms: mean of each interval, share of their latency")
        for name in REQUEST_PHASES + ["total"]:
            print(f"  {name:<14} {means[name] / 1e6:8.3f} {100 * means[name] / means['total']:6.1f}%")
    totals = requests["total"]
    print(f"latency, ms: p999 {quantile(totals, 0.999) / 1e6:.3f} max {max(totals) / 1e6:.3f}")
    values = requests[GATHER]
    print("admission to end of gathering, ms: mean p50 p90 p99 max")
    print(f"  {GATHER:<14} {sum(values) / len(values) / 1e6:8.3f} {quantile(values, 0.5) / 1e6:8.3f} "
          f"{quantile(values, 0.9) / 1e6:8.3f} {quantile(values, 0.99) / 1e6:8.3f} {max(values) / 1e6:8.3f}")


if __name__ == "__main__":
    main()
