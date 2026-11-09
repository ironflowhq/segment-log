#!/usr/bin/env python3
"""Separate diagnostic: sample task CPU ticks, including the caller threads (schedstat nanoseconds)."""
import json
import subprocess
import sys
import time
from pathlib import Path

binary, test, output = sys.argv[1:]
maximum = {}
names = {}
start = time.monotonic()
with open(Path(output) / "profile-workload.log", "w") as log:
    child = subprocess.Popen([binary, "--exact", test, "--nocapture"], stdout=log, stderr=log)
    try:
        while child.poll() is None:
            for task in Path(f"/proc/{child.pid}/task").glob("*"):
                try:
                    stat = (task / "stat").read_text()
                    name = stat[stat.index("(") + 1:stat.rindex(")")]
                    ticks = int((task / "schedstat").read_text().split()[0])
                    maximum[task.name] = max(maximum.get(task.name, 0), ticks)
                    names[task.name] = name
                except (FileNotFoundError, ProcessLookupError):
                    pass
            time.sleep(0.2)
        assert child.wait() == 0
    finally:
        if child.poll() is None:
            child.kill()
        child.wait()
elapsed = time.monotonic() - start
roles = {}
for tid, ticks in maximum.items():
    role = "callers" if names[tid].startswith("caller-") else names[tid]
    roles[role] = roles.get(role, 0) + ticks
result = {
    "scope": "diagnostic only; sampled cumulative CPU lower bounds; exited tasks can lose their last interval",
    "elapsed_seconds": elapsed, "ticks_per_second": 1000000000,
    "observed_tasks": len(maximum), "cpu_seconds_lower_bounds":
    {k: v / 1000000000 for k, v in roles.items()},
}
Path(output, "profile.json").write_text(json.dumps(result, indent=2) + "\n")
