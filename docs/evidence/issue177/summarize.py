#!/usr/bin/env python3
"""Issue 177: tables and ADR 0069's three done-when checks from run.sh's reads.txt.

Each latency is the median over rounds of one run's quantile. Check 1: random p50 <= 2x the
before scan p50 and random p99 <= 3x the before scan p99. Check 2: the scan p50 and p99 at most
10% slower than before. Check 3: the s43, s200 and callers cases fail check 1 before the change;
any other case that passes it before is reported and excluded from the benefit verdict, but must
still pass checks 1 and 2 after.
"""
import collections
import re
import statistics
import sys

MUST_FAIL_BEFORE = {"s43", "s200", "callers"}


def fields(line):
    return dict(re.findall(r"(\w+)=(\S+)", line))


runs = collections.defaultdict(list)
walks = collections.defaultdict(list)
cases, after, order = {}, {}, []
run = None
for line in open(sys.argv[1]):
    f = fields(line)
    if line.startswith("CASE "):
        cases[f["name"]] = f
        order.append(f["name"])
    elif line.startswith("RUN "):
        run = (f["name"], f["commit"], f["pattern"])
    elif line.startswith("WALK ") and run:
        walks[run[:2]].append(f)
    elif line.startswith("READ ") and run:
        runs[run].append(f)
        run = None
    elif line.startswith("AFTER "):
        after[f["name"]] = f


def med(rows, key):
    return statistics.median(float(r[key]) for r in rows)


def q(name, commit, pattern, key):
    return med(runs[(name, commit, pattern)], key)


print("| Case | Segments | Group frames per segment (median) | Log GB | Log / RAM | Resident after |")
print("| --- | ---: | ---: | ---: | ---: | ---: |")
for n in order:
    c, a = cases[n], after.get(n, {})
    gb = int(a.get("log_bytes", 0)) / 1e9
    print(f"| {n} | {c['segments']} | {int(c['median']):,} | {gb:.1f} | {a.get('log_over_ram', '-')} | {a.get('fraction', '-')} |")

print()
print("| Case | Commit | Random p50 µs | Random p99 µs | Scan p50 µs | Scan p99 µs | Scan records/s | First walks: total ms, p50 µs, max µs |")
print("| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |")
for n in order:
    for commit in ("before", "head"):
        w = walks[(n, commit)]
        print(
            f"| {n} | {commit} | {q(n, commit, 'random', 'p50_us'):,.0f} | {q(n, commit, 'random', 'p99_us'):,.0f} "
            f"| {q(n, commit, 'scan', 'p50_us'):,.0f} | {q(n, commit, 'scan', 'p99_us'):,.0f} "
            f"| {q(n, commit, 'scan', 'records_per_s'):,.0f} "
            f"| {med(w, 'total_ms'):,.0f}, {med(w, 'p50_us'):,.0f}, {med(w, 'max_us'):,.0f} |"
        )


def check1(n, commit):
    s50, s99 = q(n, "before", "scan", "p50_us"), q(n, "before", "scan", "p99_us")
    return q(n, commit, "random", "p50_us") <= 2 * s50 and q(n, commit, "random", "p99_us") <= 3 * s99


def check2(n):
    return all(q(n, "head", "scan", k) <= 1.1 * q(n, "before", "scan", k) for k in ("p50_us", "p99_us"))


print()
print("| Case | Check 1 before | Check 1 after | Check 2 | Check 3 | In benefit verdict |")
print("| --- | --- | --- | --- | --- | --- |")
passed = True
for n in order:
    b, h, s = check1(n, "before"), check1(n, "head"), check2(n)
    c3 = (not b) if n in MUST_FAIL_BEFORE else True
    passed &= h and s and c3
    mark = lambda ok: "pass" if ok else "FAIL"
    print(f"| {n} | {mark(b)} | {mark(h)} | {mark(s)} | {mark(c3)} | {'no' if b else 'yes'} |")
print()
print("VERDICT", "pass" if passed else "FAIL")
