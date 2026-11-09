#!/usr/bin/env python3
"""Summarize five comparable raw runs; no diagnostic run enters the table."""
import re
import statistics
import sys
from pathlib import Path

root = Path(sys.argv[1])
rows = []
for run in range(1, 6):
    text = (root / f"run-{run}.log").read_text()
    assert "test result: ok. 1 passed" in text
    def fields(prefix):
        line = next(line for line in text.splitlines() if line.startswith(prefix))
        return dict(re.findall(r"(\w+)=([\d.]+)", line))
    append = fields("APPEND ")
    latency = fields("METRIC append_ns ")
    device = fields("METRIC device_fdatasync_ns ")
    reopen = fields("REOPEN ")
    assert all(append[k] == "0" for k in ("full", "other_admission_errors", "commit_errors", "correctness_errors"))
    rows.append([run, float(append["records_per_second"]),
                 int(latency["p50"]) / 1e6, int(latency["p99"]) / 1e6,
                 int(latency["p999"]) / 1e6, int(device["p50"]) / 1e6,
                 int(device["p99"]) / 1e6, 1 + 2 * int(device["p99"]) / 1e6,
                 int(reopen["ns"]) / 1e6, int(reopen["end_bytes"])])
print("| Run | Records/s | Append p50 ms | p99 ms | p999 ms | Device p50 ms | p99 ms | Relative bound ms | Reopen ms | Segment bytes |")
print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
for row in rows:
    print("| " + " | ".join(str(v) if isinstance(v, int) else f"{v:.3f}" for v in row) + " |")
for name, index in [("Throughput records/s", 1), ("Append p50 ms", 2), ("Append p99 ms", 3),
                    ("Append p999 ms", 4), ("Device p50 ms", 5), ("Device p99 ms", 6), ("Reopen ms", 8)]:
    values = [r[index] for r in rows]
    print(f"\n{name}: median {statistics.median(values):.3f}, range {min(values):.3f}–{max(values):.3f}.")
