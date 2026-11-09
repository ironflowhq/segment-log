#!/usr/bin/env python3
"""Verify actual Linux calls between workload markers, not wrapper counters."""
import re
import sys
from collections import defaultdict
from pathlib import Path

phase = None
markers = []
calls = defaultdict(list)
for line in Path(sys.argv[1]).read_text().splitlines():
    marker = re.search(r'"PHASE ([a-z0-9-]+)\\n"', line)
    if marker:
        phase = marker[1]
        markers.append(phase)
    if re.search(r' (fdatasync|fsync)\(', line):
        assert phase is not None, line
        assert re.search(r'= 0 ', line), line
        calls[phase].append(line)
expected = {
    "provision": (0, None), "create": (1, 4), "commit-0": (1, 0), "commit-1": (1, 0),
    "retry": (0, 0), "drop": (0, 0), "recovery": (0, 3),
    "recovered-drop": (0, 0), "done": (0, 0),
}
assert markers == list(expected), markers
assert set(calls) <= set(expected), calls
for phase, pair in expected.items():
    actual = (sum(" fdatasync(" in x for x in calls[phase]),
              sum(" fsync(" in x for x in calls[phase]))
    if phase == "provision":
        assert actual[0] == 0 and actual[1] > 0
    else:
        assert actual == pair, (phase, actual, pair)
    print(f"{phase}: fdatasync={actual[0]} fsync={actual[1]} success")
    for line in calls[phase]:
        print(line)
