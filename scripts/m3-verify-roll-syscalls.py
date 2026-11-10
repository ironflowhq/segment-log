#!/usr/bin/env python3
"""Verify the Linux calls of a segment roll between workload markers, not wrapper counters.

Input is a strace log of acceptance::evidence::real_storage_roll_phases, taken with
-f -yy -e trace=openat,write,fdatasync,fsync so that each fd shows its path.
"""
import re
import sys
from collections import Counter, namedtuple
from pathlib import Path, PurePosixPath

PHASES = ["provision", "create", "commit-0", "roll", "commit-1", "done"]
# The workload's first group holds one record, so the roll names the new segment 1.
FIRST_SEGMENT = "00000000000000000000.seg"
NEW_SEGMENT = "00000000000000000001.seg"
HEADER_LEN = 64
HEADER_MAGIC = "SGLG"

Event = namedtuple("Event", "phase name path ret data flags line")
CALL = re.compile(
    r'^(?P<pid>\d+)\s+(?:[\d:.]+\s+)?(?P<name>openat|write|fdatasync|fsync)\((?P<args>.*)\)'
    r'\s+=\s+(?P<ret>-?\d+)'
)
UNFINISHED = re.compile(r'^(?P<pid>\d+)\s+(?P<call>.*?)\s*<unfinished \.\.\.>$')
RESUMED = re.compile(r'^(?P<pid>\d+)\s+(?:[\d:.]+\s+)?<\.\.\. \w+ resumed>(?P<tail>.*)$')
MARKER = re.compile(r'"PHASE ([a-z0-9-]+)\\n"')


class Rejected(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Rejected(message)


def join_split_calls(lines):
    """Rejoin a call that strace split across threads, at the position where it returned."""
    started = {}
    for line in lines:
        unfinished = UNFINISHED.match(line)
        resumed = RESUMED.match(line)
        if unfinished:
            started[unfinished["pid"]] = unfinished["call"]
        elif resumed and resumed["pid"] in started:
            call = started.pop(resumed["pid"])
            glue = " " if call.endswith(",") else ""
            yield f'{resumed["pid"]} {call}{glue}{resumed["tail"]}'
        else:
            yield line


def parse(text):
    events = []
    markers = []
    phase = None
    for line in join_split_calls(text.splitlines()):
        call = CALL.match(line)
        if not call:
            continue
        name, args = call["name"], call["args"]
        marker = MARKER.search(line)
        if name == "write" and marker:
            phase = marker[1]
            markers.append(phase)
            continue
        if phase is None:
            continue
        ret = int(call["ret"])
        if name == "openat":
            opened = re.match(r'[^,]+,\s*"((?:[^"\\]|\\.)*)",\s*([A-Z_|]+)', args)
            if opened:
                events.append(Event(phase, name, opened[1], ret, "", opened[2], line))
            continue
        target = re.match(r"\d+<([^>]*)>", args)
        require(target is not None, f"call without a resolved fd (strace needs -yy): {line}")
        data = re.search(r',\s*"((?:[^"\\]|\\.)*)"', args)
        events.append(Event(phase, name, target[1], ret, data[1] if data else "", "", line))
    return markers, events


def syncs(events):
    for event in events:
        if event.name in ("fdatasync", "fsync"):
            require(event.ret == 0, f"{event.phase}: sync did not return 0: {event.line}")
            yield event


def segment_writes(events):
    return [e.path for e in events if e.name == "write" and e.path.endswith(".seg")]


def position(events, found, what, after=-1):
    for index in range(after + 1, len(events)):
        if found(events[index]):
            return index
    raise Rejected(f"roll: no {what} after step {after + 1}")


def check_roll_order(events, log, new):
    created = position(
        events, lambda e: e.name == "openat" and e.path == new and "O_CREAT" in e.flags and e.ret >= 0,
        "creation of the new segment with O_CREAT")
    header = position(
        events, lambda e: e.name == "write" and e.path == new, "write to the new segment", created)
    require(events[header].data.startswith(HEADER_MAGIC) and events[header].ret == HEADER_LEN,
            f"roll: the first write to the new segment is not its header: {events[header].line}")
    data_sync = position(
        events, lambda e: e.name == "fdatasync" and e.path == new, "fdatasync of the new segment", header)
    dir_sync = position(
        events, lambda e: e.name == "fsync" and e.path == log, "fsync of the log directory", data_sync)
    frame = position(
        events, lambda e: e.name == "write" and e.path == new, "group frame write to the new segment", dir_sync)
    require(events[frame].ret > 0, f"roll: empty group frame write: {events[frame].line}")
    group_sync = position(
        events, lambda e: e.name == "fdatasync" and e.path == new, "fdatasync of the group frame", frame)
    return [created, header, data_sync, dir_sync, frame, group_sync]


def verify(text):
    markers, events = parse(text)
    require(markers == PHASES, f"phase markers {markers}, expected {PHASES}")
    by_phase = {phase: [e for e in events if e.phase == phase] for phase in PHASES}

    first_writes = segment_writes(by_phase["commit-0"])
    require(len(first_writes) == 1, f"commit-0: segment writes {first_writes}, expected one")
    first = first_writes[0]
    require(PurePosixPath(first).name == FIRST_SEGMENT, f"commit-0 wrote {first}, not {FIRST_SEGMENT}")
    log = str(PurePosixPath(first).parent)
    parent = str(PurePosixPath(log).parent)
    new = str(PurePosixPath(log) / NEW_SEGMENT)

    expected_writes = {
        "commit-0": [first], "roll": [new, new], "commit-1": [new], "done": [],
    }
    for phase, paths in expected_writes.items():
        actual = segment_writes(by_phase[phase])
        require(actual == paths, f"{phase}: segment writes {actual}, expected {paths}")

    expected_syncs = {
        "create": Counter({("fdatasync", first): 1, ("fsync", first): 1,
                           ("fsync", log): 2, ("fsync", parent): 1}),
        "commit-0": Counter({("fdatasync", first): 1}),
        "roll": Counter({("fdatasync", new): 2, ("fsync", log): 1}),
        "commit-1": Counter({("fdatasync", new): 1}),
        "done": Counter(),
    }
    report = []
    for phase in PHASES:
        done = list(syncs(by_phase[phase]))
        counts = Counter((e.name, e.path) for e in done)
        data = sum(e.name == "fdatasync" for e in done)
        full = sum(e.name == "fsync" for e in done)
        if phase == "provision":
            require(data == 0 and full > 0, f"provision: fdatasync={data} fsync={full}")
        else:
            require(counts == expected_syncs[phase], f"{phase}: syncs {dict(counts)}, expected {dict(expected_syncs[phase])}")
        report.append(f"{phase}: fdatasync={data} fsync={full} success")
        report.extend(e.line for e in done)

    order = check_roll_order(by_phase["roll"], log, new)
    steps = ["openat O_CREAT new segment", "header write", "fdatasync new segment",
             "fsync log directory", "group frame write", "fdatasync new segment"]
    report.append("roll order: " + " < ".join(steps))
    report.extend(f"  {step}: {by_phase['roll'][i].line}" for step, i in zip(steps, order))
    return report


def main():
    try:
        print("\n".join(verify(Path(sys.argv[1]).read_text())))
    except Rejected as error:
        print(f"REJECTED: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
