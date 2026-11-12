#!/usr/bin/env python3
"""Summarize an M4 evidence directory from scripts/m4-evidence.sh: usage m4-summary.py DIR.

It reads the `TAG key=value` lines that sealed_probe prints, fails on any run that reported an
error, and gives each number with its run count and range. It states no target pass: a hosted
runner is a shared virtual machine (testing.md, Benchmarks).
"""
import re
import statistics
import sys
from pathlib import Path


def lines(path, tag):
    """The key=value fields of every line in PATH that starts with TAG."""
    out = []
    for line in path.read_text().splitlines():
        if line.startswith(tag + " "):
            out.append(dict(re.findall(r"(\w+)=(\S+)", line)))
    return out


def one(path, tag):
    found = lines(path, tag)
    assert found, f"{path} has no {tag} line"
    return found[-1]


def ms(ns):
    return int(ns) / 1e6


def spread(values, digits=3):
    return f"median {statistics.median(values):.{digits}f}, range {min(values):.{digits}f} to {max(values):.{digits}f}"


def max_rss_kib(path):
    match = re.search(r"Maximum resident set size \(kbytes\): (\d+)", path.read_text())
    return int(match.group(1)) if match else None


def fixtures(root):
    print("## Fixtures\n")
    print("| Fixture | Records | Sealed segments | Records per sealed segment | Record entry bytes | Active bytes | Active records | Directory bytes | Expected filter bytes | Build s |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    for name in ["after-keyed", "after-unkeyed", "before-keyed"]:
        path = root / f"fixture-{name}.log"
        f, k, b = one(path, "FIXTURE"), one(path, "FILTERS"), one(path, "BUILD")
        per = f["min_records_per_sealed"]
        if f["max_records_per_sealed"] != per:
            per += f" to {f['max_records_per_sealed']}"
        # The M3 code writes no key frames, so the formula does not apply to its fixture.
        filter_bytes = "none in M3" if name.startswith("before") else k["expected_filter_bytes"]
        print(f"| {name} | {f['records']} | {f['sealed_segments']} | {per} | {f['record_entry_bytes']} | {f['active_bytes']} | {f['active_records']} | {f['dir_bytes']} | {filter_bytes} | {int(b['ns']) / 1e9:.1f} |")
    print()


def opens(root):
    print("## Open time\n")
    print("Each run opens the after-side fixture once with `SegmentLog::open`, end to end. Target: 50 ms (docs/prd.md). Cold runs drop the page cache before each open; `page_cache_before_kib` shows the drop.\n")
    print("| Fixture | Cache | Runs | Min ms | p50 ms | Max ms | Page cache before, KiB |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    p50 = {}
    for fixture in ["keyed", "unkeyed"]:
        for cache in ["cold", "warm"]:
            path = root / "open" / f"{fixture}-{cache}.log"
            if not path.exists():
                print(f"| {fixture} | {cache} | not run | | | | |")
                continue
            runs = lines(path, "OPEN")
            s = one(path, "OPEN_SUMMARY")
            assert s["correctness_errors"] == "0" and all(r["correctness_errors"] == "0" for r in runs), path
            cached = [r["page_cache_before_kib"] for r in runs]
            cached = "na" if "na" in cached else f"{min(map(int, cached))} to {max(map(int, cached))}"
            p50[fixture, cache] = ms(s["p50_ns"])
            print(f"| {fixture} | {cache} | {s['runs']} | {ms(s['min_ns']):.3f} | {ms(s['p50_ns']):.3f} | {ms(s['max_ns']):.3f} | {cached} |")
    print()
    for cache in ["cold", "warm"]:
        if ("keyed", cache) in p50 and ("unkeyed", cache) in p50:
            print(f"Keyed minus unkeyed p50, {cache}: {p50['keyed', cache] - p50['unkeyed', cache]:.3f} ms. The fixtures differ in their keys, their key frames and 16 bytes per record.\n")
    note = root / "open" / "cold-not-applicable.txt"
    if note.exists():
        print(note.read_text().strip() + ".\n")


def memory(root):
    print("## Key-cache memory\n")
    keyed = root / "open" / "keyed-warm.log"
    unkeyed = root / "open" / "unkeyed-warm.log"
    filters, fixture = one(keyed, "FILTERS"), one(keyed, "FIXTURE")
    k, u = lines(keyed, "OPEN")[0], lines(unkeyed, "OPEN")[0]
    expected = int(filters["expected_filter_bytes"])
    held = int(k["heap_retained_bytes"]) - int(u["heap_retained_bytes"])
    print(f"- ADR 0048 rule 5, the filter part: {expected} bytes for {filters['sealed_keys']} sealed keys ({filters['filter_bytes_per_key']} bytes per key), from the key filter frame layout in format.md.")
    print(f"- Heap the open log holds (allocations live after open, first run): keyed {k['heap_retained_bytes']} bytes, unkeyed {u['heap_retained_bytes']} bytes. The difference, {held} bytes, is the measured key-cache memory: the filters, the index-page fences and the active segment's exact map of {fixture['active_keys']} keys.")
    print(f"- Measured minus the filter formula: {held - expected} bytes, {(held - expected) / max(int(fixture['active_keys']), 1):.1f} bytes per active-segment key.")
    print(f"- Peak heap during open: keyed {k['heap_peak_bytes']} bytes, unkeyed {u['heap_peak_bytes']} bytes.")
    print(f"- Resident set before and after the first open, KiB: keyed {k['rss_before_kib']} to {k['rss_after_kib']}, unkeyed {u['rss_before_kib']} to {u['rss_after_kib']}.\n")


def appends(root):
    print("## Appends, M3 (before) and M4 (after)\n")
    medians = {}
    for mode in ["closed", "closed-unkeyed", "paced"]:
        directory = root / "append" / mode
        if not directory.exists():
            continue
        runs = {"before": [], "after": []}
        for path in sorted(directory.glob("*-*.log")):
            side, run = path.stem.split("-")
            a, m, c, o = one(path, "APPEND"), one(path, "METRIC append_ns"), one(path, "CONFIG"), one(path, "OPEN")
            assert a["commit_errors"] == "0" and a["correctness_errors"] == "0", path
            assert m["count"] == a["records"], path
            runs[side].append((int(run), a, m, c, o, max_rss_kib(path)))
        c = runs["after"][0][3]
        keys = "keyed" if c.get("appends_keyed", "1") == "1" else "unkeyed"
        print(f"### {mode}: {keys} appends, {c['callers']} callers, {c['rounds']} rounds, interval {c['interval_us']} µs, offered {c['offered_records_per_second']} records/s, roll_bytes {c['roll_bytes']}, start {c['start_records']} records in {c['start_segments']} segments\n")
        print("| Side | Run | Records/s | p50 ms | p99 ms | p999 ms | Max ms | Full | Segments | Open ms | Max RSS KiB |")
        print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
        for side in ["before", "after"]:
            for run, a, m, _, o, rss in sorted(runs[side], key=lambda r: r[0]):
                print(f"| {side} | {run} | {float(a['records_per_second']):.0f} | {ms(m['p50']):.3f} | {ms(m['p99']):.3f} | {ms(m['p999']):.3f} | {ms(m['max']):.3f} | {a['full']} | {a['start_segments']} to {a['end_segments']} | {ms(o['ns']):.1f} | {rss or 'na'} |")
        print()
        for label, get in [("Records/s", lambda r: float(r[1]["records_per_second"])), ("p50 ms", lambda r: ms(r[2]["p50"])), ("p99 ms", lambda r: ms(r[2]["p99"]))]:
            before = [get(r) for r in runs["before"]]
            after = [get(r) for r in runs["after"]]
            overlap = max(min(before), min(after)) <= min(max(before), max(after))
            ratio = statistics.median(after) / statistics.median(before)
            print(f"{label}: before {spread(before)}; after {spread(after)}; after/before medians {ratio:.3f}; ranges overlap: {'yes' if overlap else 'no'}.\n")
            if label == "Records/s":
                medians[mode] = {"before": statistics.median(before), "after": statistics.median(after)}
    if "closed" in medians and "closed-unkeyed" in medians:
        print("### Key lookup cost on each side\n")
        print("Both modes append to the same keyed fixture. Unkeyed appends skip the key lookup, so the gap is what the lookup costs on that side (ADR 0051).\n")
        for side in ["before", "after"]:
            keyed, unkeyed = medians["closed"][side], medians["closed-unkeyed"][side]
            extra_us = (1 / keyed - 1 / unkeyed) * 1e6
            print(f"- {side}: keyed/unkeyed medians {keyed / unkeyed:.3f}; keyed minus unkeyed time per record {extra_us:.2f} µs.")
        print()


def main(root):
    print("# M4 evidence summary\n")
    print("Conditions are in environment.txt. A hosted runner is a shared virtual machine: these are measurements under those conditions, not a target pass.\n")
    fixtures(root)
    opens(root)
    memory(root)
    appends(root)


if __name__ == "__main__":
    main(Path(sys.argv[1]))
