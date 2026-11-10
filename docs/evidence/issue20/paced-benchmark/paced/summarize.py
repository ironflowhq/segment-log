#!/usr/bin/env python3
"""Tabulate retained paced benchmark logs, one row for each run."""
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
print("| Run | Offered records/s | Records/s | Mean ms | p50 ms | p99 ms | p999 ms | Max ms | Bound ms | Mean flush µs | Groups |")
print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
for prefix in sys.argv[2:]:
    for path in sorted(root.glob(f"{prefix}-*.log")):
        text = path.read_text()
        assert "test result: ok. 1 passed; 0 failed" in text, path
        assert "full=0 other_admission_errors=0 commit_errors=0 correctness_errors=0" in text, path

        def fields(marker):
            line = next(line for line in text.splitlines() if line.startswith(marker))
            return dict(re.findall(r"(\w+)=([0-9.]+)", line))

        paced, append, latency = fields("PACED "), fields("APPEND "), fields("METRIC append_ns ")
        device, config, groups = fields("METRIC device_fdatasync_ns "), fields("CONFIG "), fields("METRIC group_submits ")
        flush = fields("STORAGE kind=Group.SyncData ")
        ms = lambda name: f"{int(latency[name]) / 1e6:.3f}"
        print(f"| {path.stem} | {float(paced['offered_records_per_second']):,.0f} | {float(append['records_per_second']):,.0f} | "
              f"{int(latency['sum']) / int(latency['count']) / 1e6:.3f} | {ms('p50')} | {ms('p99')} | {ms('p999')} | {ms('max')} | "
              f"{(int(config['linger_ns']) + 2 * int(device['p99'])) / 1e6:.3f} | "
              f"{int(flush['sum_ns']) / int(flush['count']) / 1e3:.0f} | {int(groups['count']):,} |")
