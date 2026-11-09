#!/usr/bin/env python3
"""Compare requested heap allocation across identical histories, alternating binaries."""
import argparse
import csv
import io
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binaries", nargs="+", help="LABEL=PATH for each compiled memory_probe")
parser.add_argument("--records", type=int, default=512)
parser.add_argument("--repetitions", type=int, default=5)
parser.add_argument("--data", type=Path, required=True, help="new directory reused after each successful run")
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
assert args.records >= 64 and args.records % 64 == 0
assert args.repetitions > 0
n = args.records
# Each pair changes one history dimension while holding the others fixed.
cases = {
    "base": (n, 0, 1, n // 16, 32),
    "records_quarter": (n // 4, 0, 1, n // 64, 32),
    "keys_half": (n, n // 2, 1, n // 16, 32),
    "keys_all": (n, n, 1, n // 16, 32),
    "streams_64": (n, 0, 64, n // 16, 32),
    "groups_64": (n, 0, 1, n // 64, 32),
    "groups_per_record": (n, 0, 1, 1, 32),
    "payload_256": (n, 0, 1, n // 16, 256),
}
with args.output.open("w", newline="") as output:
    writer = csv.writer(output)
    writer.writerow(["variant", "case", "repetition", "records", "keys", "streams", "group_records", "payload_bytes", "phase", "thread_allocated_bytes", "process_live_delta_bytes", "process_peak_extra_bytes", "process_live_bytes"])
    for case, dimensions in cases.items():
        for repetition in range(1, args.repetitions + 1):
            for binary in args.binaries:
                label, path = binary.split("=", 1)
                result = subprocess.run([path, str(args.data), *map(str, dimensions)], text=True, capture_output=True, check=True)
                lines = result.stdout.splitlines()
                expected = f"validated_records={dimensions[0]},failures=0,groups={(dimensions[0] + dimensions[3] - 1) // dimensions[3]}"
                assert lines[-1] == expected, result.stdout
                rows = list(csv.reader(io.StringIO("\n".join(lines[:-1]))))
                assert len(rows) == 12, result.stdout
                for row in rows[1:]:
                    writer.writerow([label, case, repetition, *dimensions, *row])
                output.flush()
