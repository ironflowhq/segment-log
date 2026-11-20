#!/usr/bin/env python3
"""Issue 177: tables and the done-when checks of ADR 0069, as ADR 0070 amends them, from one or
more reads.txt files that run.sh wrote.

    summarize.py READS.txt [READS.txt ...]

Each latency is the median over rounds of one run's quantile. A case is one log: its name and
segment count. Check 1: random p50 <= 2x the before scan p50, and random p99 <= 3x the before scan
p99. Check 2: the scan p50 and p99 at most 10% slower than before. Check 3: the s43, s200 and
callers shapes fail check 1 before the change. ADR 0070: a log that is not fully resident (a
resident fraction below 0.99) and fails check 1 is judged by a fully resident log of the same name;
checks 2 and 3 apply to every log.
"""
import collections
import re
import statistics
import sys

MUST_FAIL_BEFORE = {"s43", "s200", "callers"}
RESIDENT = 0.99


def fields(line):
    return dict(re.findall(r"(\w+)=(\S+)", line))


runs = collections.defaultdict(list)
walks = collections.defaultdict(list)
cases, order = {}, []
for path in sys.argv[1:]:
    run, label = None, {}
    for line in open(path):
        f = fields(line)
        if line.startswith("CASE "):
            key = (f["name"], int(f["segments"]))
            label[f["name"]] = key
            cases[key] = f
            order.append(key)
        elif line.startswith("RUN "):
            run = (label[f["name"]], f["commit"], f["pattern"])
        elif line.startswith("WALK ") and run:
            walks[run[:2]].append(f)
        elif line.startswith("READ ") and run:
            runs[run].append(f)
            run = None
        elif line.startswith("AFTER "):
            cases[label[f["name"]]].update(after=f)


def med(rows, key):
    return statistics.median(float(r[key]) for r in rows)


def q(k, commit, pattern, key):
    return med(runs[(k, commit, pattern)], key)


def name(k):
    return f"{k[0]} ({k[1]} segments)"


def resident(k):
    return float(cases[k]["after"]["fraction"]) >= RESIDENT


print("| Log | Group frames per segment (median) | Log GB | Log / RAM | Resident after |")
print("| --- | ---: | ---: | ---: | ---: |")
for k in order:
    c, a = cases[k], cases[k]["after"]
    print(f"| {name(k)} | {int(c['median']):,} | {int(a['log_bytes']) / 1e9:.1f} | {a['log_over_ram']} | {a['fraction']} |")

print()
print("| Log | Commit | Random p50 µs | Random p99 µs | Scan p50 µs | Scan p99 µs | Scan records/s | First walks: total ms, p50 µs, max µs |")
print("| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |")
for k in order:
    for commit in ("before", "head"):
        w = walks[(k, commit)]
        print(
            f"| {name(k)} | {commit} | {q(k, commit, 'random', 'p50_us'):,.0f} | {q(k, commit, 'random', 'p99_us'):,.0f} "
            f"| {q(k, commit, 'scan', 'p50_us'):,.0f} | {q(k, commit, 'scan', 'p99_us'):,.0f} "
            f"| {q(k, commit, 'scan', 'records_per_s'):,.0f} "
            f"| {med(w, 'total_ms'):,.0f}, {med(w, 'p50_us'):,.0f}, {med(w, 'max_us'):,.0f} |"
        )


def check1(k, commit):
    s50, s99 = q(k, "before", "scan", "p50_us"), q(k, "before", "scan", "p99_us")
    return q(k, commit, "random", "p50_us") <= 2 * s50 and q(k, commit, "random", "p99_us") <= 3 * s99


def check2(k):
    return all(q(k, "head", "scan", s) <= 1.1 * q(k, "before", "scan", s) for s in ("p50_us", "p99_us"))


mark = lambda ok: "pass" if ok else "FAIL"
print()
print("| Log | Check 1 before | Check 1 after | Judged by | Check 2 | Check 3 | In benefit verdict |")
print("| --- | --- | --- | --- | --- | --- | --- |")
passed = True
for k in order:
    b, h = check1(k, "before"), check1(k, "head")
    judge = k
    if not h and not resident(k):
        judge = next((j for j in order if j[0] == k[0] and j != k and resident(j)), None)
    ok1 = h if judge == k else judge is not None and check1(judge, "head")
    c3 = (not b) if k[0] in MUST_FAIL_BEFORE else True
    passed &= ok1 and check2(k) and c3
    by = "itself" if judge == k else (name(judge) if judge else "no resident log")
    print(f"| {name(k)} | {mark(b)} | {mark(h)} | {by}: {mark(ok1)} | {mark(check2(k))} | {mark(c3)} | {'no' if b else 'yes'} |")
print()
print("VERDICT", "pass" if passed else "FAIL")
