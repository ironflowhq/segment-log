#!/usr/bin/env python3
"""Wait for a quiet Linux guest before a timing run: usage m1-quiet.py FILE.

Other work shares the guest's processors and disk. This waits, at most three minutes, for two
seconds in a row with less than a tenth of the processor time busy. It appends what it saw to
FILE either way, so a report can label a run that started on a busy guest.
"""
import sys
import time


def snapshot():
    fields = [int(x) for x in open("/proc/stat").readline().split()[1:9]]
    return sum(fields), fields[3] + fields[4]


def busy_percent(before, after):
    """Share of processor time between two (total, idle) readings that was not idle."""
    total, idle = after[0] - before[0], after[1] - before[1]
    return 100 * (1 - idle / total) if total > 0 else 0.0


def main(path):
    quiet = waited = 0
    busy = 0.0
    while quiet < 2 and waited < 180:
        before = snapshot()
        time.sleep(1)
        busy = busy_percent(before, snapshot())
        quiet = quiet + 1 if busy < 10 else 0
        waited += 1
    with open(path, "a") as out:
        out.write(f"guest_busy_percent_before={busy:.1f} quiet_seconds={quiet} waited_seconds={waited}\n")


if __name__ == "__main__":
    main(sys.argv[1])
