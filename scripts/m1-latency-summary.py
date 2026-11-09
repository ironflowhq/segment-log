#!/usr/bin/env python3
import re
import statistics
import sys
from pathlib import Path

root = Path(sys.argv[1])
for prefix in sys.argv[2:]:
    rows = []
    for run in range(1, 6):
        text = (root / f"{prefix}-{run}.log").read_text()
        assert "test result: ok. 1 passed; 0 failed" in text
        def fields(marker):
            line = next(line for line in text.splitlines() if line.startswith(marker))
            return dict(re.findall(r"(\w+)=([0-9.]+)", line))
        append = fields("APPEND ")
        assert int(append["records"]) == 1_000_000
        for key in ["full", "other_admission_errors", "commit_errors", "correctness_errors"]:
            assert int(append[key]) == 0, (prefix, run, key)
        reopen = fields("REOPEN ")
        assert int(reopen["correctness_errors"]) == 0
        assert int(reopen["end_records"]) == 1_000_000
        assert reopen["start_bytes"] == reopen["end_bytes"]
        latency = fields("METRIC append_ns ")
        device = fields("METRIC device_fdatasync_ns ")
        config = fields("CONFIG ")
        assert int(latency["count"]) == 1_000_000
        rows.append([
            float(append["records_per_second"]),
            int(latency["p50"]) / 1e6,
            int(latency["p99"]) / 1e6,
            int(latency["p999"]) / 1e6,
            (int(config["linger_ns"]) + 2 * int(device["p99"])) / 1e6,
            int(reopen["ns"]) / 1e6,
        ])
    print(f"## {prefix}\n")
    print("| Run | Records/s | p50 ms | p99 ms | p999 ms | Relative bound ms | Reopen ms |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for run, row in enumerate(rows, 1):
        print(f"| {run} | " + " | ".join(f"{x:.3f}" for x in row) + " |")
    print()
    for label, values in zip(["Records/s", "p50 ms", "p99 ms", "p999 ms", "Relative bound ms", "Reopen ms"], zip(*rows)):
        print(f"{label}: median {statistics.median(values):.3f}, range {min(values):.3f} to {max(values):.3f}.\n")
