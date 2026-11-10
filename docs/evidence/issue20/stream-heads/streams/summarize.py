#!/usr/bin/env python3
"""Tabulate retained growth_probe output: append and reopen time by stream count."""
import csv
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
print("| Streams | Binary | Append ms, three runs | Peak extra heap bytes while appending | Reopen ms, three runs |")
print("| ---: | --- | --- | ---: | --- |")
for count in [1, 1000, 10000, 100000]:
    for binary in sys.argv[2:]:
        append, reopen, peak = [], [], set()
        for path in sorted(root.glob(f"{binary}-{count}-*.txt")):
            lines = path.read_text().splitlines()
            assert lines[-1] == "validated_records=200000,failures=0,groups=2000", path
            rows = {row[0]: row for row in csv.reader(lines[:-1])}
            append.append(int(rows["append_and_release_caller"][5]) / 1e6)
            peak.add(int(rows["append_and_release_caller"][3]))
            reopen.append(int(rows["reopen"][5]) / 1e6)
        assert len(append) == 3 and len(peak) == 1, (binary, count)
        print(f"| {count:,} | {binary} | " + ", ".join(f"{x:,.0f}" for x in append) + f" | {peak.pop():,} | "
              + ", ".join(f"{x:.0f}" for x in reopen) + " |")
