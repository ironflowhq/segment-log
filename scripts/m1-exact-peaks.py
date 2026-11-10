#!/usr/bin/env python3
"""Exact resident peaks for each phase, and the bytes that the engine owns, on Linux.

`peaks` replays the kernel's `kmem:rss_stat` events for each resident_probe process. Every change
to a process's resident counters emits one event that carries the exact new value of the changed
counter, so the replay knows the resident set at every change, not only at the barriers. At each
barrier the replay must equal `VmRSS`, which the kernel also sums exactly. The kernel's high-water
mark (`VmHWM`) is read beside it, with and without pinning the probe to one processor.

`owners` runs owner_probe, which gives the engine its own allocator arena and prints the
engine's bytes apart from the caller's and the test's at each barrier.

Needs root, tracefs at /sys/kernel/tracing and the host's process ids (`docker run --pid=host`),
because the trace names processes by their ids outside any namespace.
"""
import argparse
import csv
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys
import threading
import time

_spec = importlib.util.spec_from_file_location("peaks", Path(__file__).with_name("m1-linux-phase-peaks.py"))
peaks = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(peaks)

DIMENSIONS = peaks.DIMENSIONS
PHASES = {**peaks.PHASES,
          "pulse": {"baseline": peaks.START, "pulsed": "allocate, fill, check and drop one buffer"}}
# The members that make up the resident set. Swap entries are traced too but are not resident.
RESIDENT = ("MM_FILEPAGES", "MM_ANONPAGES", "MM_SHMEMPAGES")
STATUS_MEMBER = {"MM_FILEPAGES": "RssFile", "MM_ANONPAGES": "RssAnon", "MM_SHMEMPAGES": "RssShmem"}
EVENT = re.compile(r"\[(\d+)\]\s+\S+\s+(\d+\.\d+): rss_stat: mm_id=(\d+) curr=([01]) type=(MM_\w+) size=(-?\d+)B$")
# The known transient: one buffer, resident only inside its phase.
PULSE = dict(records=16384, keys=0, streams=1, group=1, payload=1024)
# glibc maps a buffer this size with one header page beside it.
PULSE_SLACK_KB = 8
SNAPSHOT_TRIES = 50
OWNER_PHASES = {
    "append": ["baseline", "empty_held", "caller_held", "appended_held", "read_one_held", "read_one_dropped",
               "validation_held", "validation_dropped", "engine_dropped"],
    "recover": ["baseline", "recovered_held", "read_one_held", "read_one_dropped", "validation_held",
                "validation_dropped", "engine_dropped"],
}
OWNER_COLUMNS = ("engine_live_bytes", "engine_peak_bytes", "engine_bytes_caller_holds", "engine_held_bytes",
                 "other_live_bytes", "other_peak_bytes", "arena_resident_bytes", "writer_stack_resident_bytes")
CALIBRATION_BYTES = 1 << 20
# Each known-size check that owner_probe prints, and the value it must observe exactly.
OWNER_CHECKS = {"pages_touched": CALIBRATION_BYTES, "pages_unmapped": 0, "engine_buffer": CALIBRATION_BYTES,
                "engine_release": 0, "other_buffer": CALIBRATION_BYTES, "other_release": 0}


def parse_events(text):
    """Return (cpu, timestamp, mm_id, curr, member, bytes) for each rss_stat line. Any other line is an error."""
    events = []
    for line in text.splitlines():
        if not line.strip():
            continue
        match = EVENT.search(line)
        if not match:
            raise ValueError(f"trace: unexpected line {line!r}")
        cpu, stamp, mm, curr, member, size = match.groups()
        events.append((int(cpu), float(stamp), int(mm), int(curr), member, int(size)))
    return events


def replay(start, events):
    """Apply events to the resident members of `start`. Return the end members and the peak set.

    `start` maps each member to bytes. The peak includes the start, since a phase cannot peak
    below the resident set it starts with.
    """
    values = dict(start)
    total = lambda: sum(values[member] for member in RESIDENT)
    peak, at = total(), None
    for index, (_, _, _, _, member, size) in enumerate(events):
        values[member] = size
        if member in RESIDENT and total() > peak:
            peak, at = total(), index
    return values, peak, at


def status_members(status):
    return {member: status[field] * 1024 for member, field in STATUS_MEMBER.items()} | {"MM_SWAPENTS": status["VmSwap"] * 1024}


class Tracer:
    """Two trace instances: one for the probe's own threads, one for changes made from other contexts."""

    def __init__(self, root=Path("/sys/kernel/tracing"), name="seglog-m1-exact"):
        self.own, self.remote = root / "instances" / f"{name}-own", root / "instances" / f"{name}-remote"
        # Kilobytes for each processor. A phase that overflows its buffer fails the loss check.
        self.buffer_kb = {self.own: 16384, self.remote: 4096}

    def write(self, path, text):
        path.write_text(text)

    def open(self):
        for instance in (self.own, self.remote):
            instance.mkdir()
            # One clock for every processor, so that the two instances merge in order.
            self.write(instance / "trace_clock", "mono")
            self.write(instance / "buffer_size_kb", str(self.buffer_kb[instance]))
        self.write(self.own / "options" / "event-fork", "1")
        # A change made while the process's own memory is not current: reclaim, khugepaged, another process.
        self.write(self.remote / "events" / "kmem" / "rss_stat" / "filter", "curr == 0")

    def start(self, pid):
        self.write(self.own / "set_event_pid", str(pid))
        for instance in (self.own, self.remote):
            self.write(instance / "events" / "kmem" / "rss_stat" / "enable", "1")

    def drain(self):
        """Read and consume everything buffered so far, without waiting for more."""
        texts = []
        for instance in (self.own, self.remote):
            fd = os.open(instance / "trace_pipe", os.O_RDONLY | os.O_NONBLOCK)
            chunks = []
            try:
                while True:
                    try:
                        chunk = os.read(fd, 1 << 20)
                    except BlockingIOError:
                        break
                    if not chunk:
                        break
                    chunks.append(chunk)
            finally:
                os.close(fd)
            texts.append(b"".join(chunks).decode())
        return texts

    def losses(self):
        """Events that an instance overwrote or dropped, summed over its processors."""
        lost = {}
        for instance in (self.own, self.remote):
            count = 0
            for stats in sorted((instance / "per_cpu").glob("cpu*/stats")):
                for line in stats.read_text().splitlines():
                    name, _, value = line.partition(":")
                    if name in ("overrun", "commit overrun", "dropped events"):
                        count += int(value)
            lost[instance.name] = count
        return lost

    def stop(self):
        for instance in (self.own, self.remote):
            self.write(instance / "events" / "kmem" / "rss_stat" / "enable", "0")
        # Opening for writing truncates the list, so an empty write clears it.
        self.write(self.own / "set_event_pid", "")
        self.drain()

    def close(self):
        for instance in (self.own, self.remote):
            if instance.exists():
                self.write(instance / "events" / "kmem" / "rss_stat" / "enable", "0")
                instance.rmdir()


def snapshot(proc, tracer, events):
    """Read `status` while no counter changes. Return it and every event up to that read.

    A change between the two drains would make the reading and the events disagree, so the read
    is repeated until the drain after it is empty.
    """
    own, remote = tracer.drain()
    events["own"] += own
    events["remote"] += remote
    for _ in range(SNAPSHOT_TRIES):
        status = (proc / "status").read_text()
        own, remote = tracer.drain()
        events["own"] += own
        events["remote"] += remote
        if not own and not remote:
            return status
        time.sleep(0.01)
    raise ValueError("the counters did not settle at a barrier")


def phase_trace(start, own_text, remote_text, mm_id):
    """Merge the two instances in time order for one phase and replay them."""
    own = parse_events(own_text)
    mm_ids = {event[2] for event in own}
    if mm_id is None and own:
        mm_id = own[0][2]
    if mm_ids - {mm_id}:
        raise ValueError(f"trace: the process's threads changed more than one memory: {sorted(mm_ids)}")
    if any(event[3] != 1 for event in own):
        raise ValueError("trace: a thread of the process changed a memory that was not its own")
    remote = [event for event in parse_events(remote_text) if event[2] == mm_id]
    merged = sorted([(e[1], 0, i, e) for i, e in enumerate(own)] + [(e[1], 1, i, e) for i, e in enumerate(remote)])
    events = [entry[3] for entry in merged]
    end, peak, at = replay(start, events)
    return {"events": len(own), "remote_events": [list(e) for e in remote], "start": start, "end": end,
            "peak_bytes": peak, "peak_event": at,
            "first_stamp": events[0][1] if events else None, "last_stamp": events[-1][1] if events else None,
            "compact": "".join(f"{e[1]:.6f} {e[0]} {e[3]} {e[4]} {e[5]}\n" for e in events)}, mm_id


def run_trial(binary, mode, data, history, tracer, cpu=None, timeout=600, proc=lambda pid: Path(f"/proc/{pid}")):
    """Run one probe process under the trace. Keep every reading, also when the trial fails."""
    command = (["taskset", "-c", str(cpu)] if cpu is not None else []) + \
        [str(binary), mode, str(data), *(str(history[name]) for name in DIMENSIONS)]
    trial = {"command": command, "mode": mode, "history": history, "cpu": cpu, "events": [],
             "readings": [], "error": None, "compact": ""}
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True)
    watchdog = threading.Timer(timeout, process.kill)
    watchdog.start()
    work, values, mm_id = None, None, None
    try:
        for line in process.stdout:
            parts = line.rstrip("\n").split("\t")
            trial["events"].append(parts)
            if parts[0] == "end":
                work = parts
                continue
            if parts[0] != "barrier":
                continue
            here = proc(process.pid)
            reading = {"phase": parts[1], "work": work and work[1], "work_ns": work and int(work[3])}
            if values is None:
                reading["status"] = (here / "status").read_text()
                reading["cpus_allowed"] = re.search(r"^Cpus_allowed_list:\s*(\S+)", reading["status"], re.M)[1]
                values = status_members(peaks.parse_status(reading["status"]))
                tracer.start(process.pid)
                reading["trace"] = None
            else:
                events = {"own": "", "remote": ""}
                reading["status"] = snapshot(here, tracer, events)
                traced, mm_id = phase_trace(values, events["own"], events["remote"], mm_id)
                trial["compact"] += f"# {parts[1]}\n" + traced.pop("compact")
                values = traced["end"]
                reading["trace"] = traced
            reading["smaps_rollup"] = (here / "smaps_rollup").read_text()
            (here / "clear_refs").write_text("5")
            reading["status_after_reset"] = (here / "status").read_text()
            trial["readings"].append(reading)
            work = None
            process.stdin.write("go\n")
            process.stdin.flush()
    except (OSError, ValueError, IndexError, TypeError) as error:
        trial["error"] = repr(error)
        process.kill()
    finally:
        watchdog.cancel()
    trial["stderr"] = process.stderr.read()
    trial["returncode"] = process.wait()
    for pipe in (process.stdin, process.stdout, process.stderr):
        pipe.close()
    trial["mm_id"] = mm_id
    trial["losses"] = tracer.losses()
    tracer.stop()
    return trial


def check(trial):
    """Return why a trial cannot be used. An empty list means it can."""
    problems = [trial["error"]] if trial["error"] else []
    if trial["returncode"] != 0:
        problems.append(f"exit status {trial['returncode']}: {trial['stderr'].strip()}")
    expected = list(PHASES[trial["mode"]])
    seen = [reading["phase"] for reading in trial["readings"]]
    if seen != expected:
        problems.append(f"barriers {seen}, expected {expected}")
    records = trial["history"]["records"]
    appended = records if trial["mode"] == "append" else 0
    want = {"accepted": appended, "committed": appended, "admission_errors": 0, "commit_errors": 0,
            "validated": records if trial["mode"] in ("append", "recover") else 0, "failures": 0}
    results = [dict(part.split("=") for part in event[1:]) for event in trial["events"] if event[0] == "result"]
    if results != [{name: str(value) for name, value in want.items()}]:
        problems.append(f"result {results}, expected {want}")
    if any(trial.get("losses", {}).values()):
        problems.append(f"the trace lost events: {trial['losses']}")
    if trial["readings"] and trial["cpu"] is not None and trial["readings"][0]["cpus_allowed"] != str(trial["cpu"]):
        problems.append(f"pinned to {trial['readings'][0]['cpus_allowed']}, expected {trial['cpu']}")
    for reading in trial["readings"]:
        try:
            status = peaks.parse_status(reading["status"])
            peaks.parse_status(reading["status_after_reset"])
            peaks.parse_rollup(reading["smaps_rollup"])
        except ValueError as error:
            problems.append(f"{reading['phase']}: {error}")
            continue
        traced = reading.get("trace")
        exact = status_members(status)
        if traced and any(traced["end"][member] != exact[member] for member in RESIDENT):
            problems.append(f"{reading['phase']}: the replay ends at {traced['end']}, status says {exact}")
    return problems


def rows(trial):
    """One row for each barrier. Sizes in KiB; every traced value is a whole number of pages."""
    out = []
    for order, reading in enumerate(trial["readings"]):
        status = peaks.parse_status(reading["status"])
        rollup = peaks.parse_rollup(reading["smaps_rollup"])
        traced = reading["trace"]
        start = out[-1]["steady_kb"] if out else status["VmRSS"]
        peak = traced["peak_bytes"] // 1024 if traced else status["VmRSS"]
        baseline = out[0]["steady_kb"] if out else status["VmRSS"]
        out.append({"history": trial.get("name", ""), **trial["history"], "mode": trial["mode"],
                    "pinned": trial["cpu"] is not None, "repetition": trial.get("repetition", 0),
                    "order": order, "phase": reading["phase"], "work": reading["work"] or "",
                    "work_ns": reading["work_ns"] or "", "events": traced["events"] if traced else 0,
                    "remote_events": len(traced["remote_events"]) if traced else 0,
                    "start_kb": start, "steady_kb": status["VmRSS"], "peak_kb": peak,
                    "mark_kb": status["VmHWM"], "mark_low_kb": peak - status["VmHWM"],
                    "transient_kb": peak - max(start, status["VmRSS"]),
                    "anon_kb": status["RssAnon"], "file_kb": status["RssFile"],
                    "anon_huge_kb": rollup["AnonHugePages"], "rollup_rss_kb": rollup["Rss"],
                    "steady_above_baseline_kb": status["VmRSS"] - baseline,
                    "peak_above_baseline_kb": peak - baseline})
    return out


ROW_COLUMNS = ("history", *DIMENSIONS, "mode", "pinned", "repetition", "order", "phase", "work", "work_ns",
               "events", "remote_events", "start_kb", "steady_kb", "peak_kb", "mark_kb", "mark_low_kb",
               "transient_kb", "anon_kb", "file_kb", "anon_huge_kb", "rollup_rss_kb",
               "steady_above_baseline_kb", "peak_above_baseline_kb")


def check_pulse(trial):
    """Return why the replay fails the known transient: a buffer resident only inside one phase."""
    problems = check(trial)
    if problems:
        return problems
    baseline, pulsed = rows(trial)
    size = trial["history"]["records"] * trial["history"]["payload"] // 1024
    if not size <= pulsed["transient_kb"] <= size + PULSE_SLACK_KB:
        problems.append(f"a {size} KiB pulse replayed as a {pulsed['transient_kb']} KiB transient")
    if abs(pulsed["steady_kb"] - baseline["steady_kb"]) > PULSE_SLACK_KB:
        problems.append("the pulse stayed resident after its phase")
    return problems


def owner_rows(text):
    """Parse owner_probe output: a header, one row for each barrier, check lines, then a result line."""
    lines = text.strip().splitlines()
    if not lines or lines[0].split(",") != ["phase", *OWNER_COLUMNS]:
        raise ValueError("owner probe: unexpected header")
    out = []
    for line in lines[1:-1]:
        if line.startswith("check,"):
            continue
        phase, *values = line.split(",")
        if len(values) != len(OWNER_COLUMNS):
            raise ValueError(f"owner probe: malformed row {line!r}")
        out.append({"phase": phase, **{name: None if value == "-" else int(value)
                                       for name, value in zip(OWNER_COLUMNS, values)}})
    return out, lines[-1]


def owner_checks(text):
    """The known-size checks: name to (expected, observed)."""
    found = {}
    for line in text.splitlines():
        if line.startswith("check,"):
            _, name, expected, observed = line.split(",")
            if name in found:
                raise ValueError(f"owner probe: repeated check {name}")
            found[name] = (int(expected), int(observed))
    return found


def check_owner(trial):
    """Return why an owner trial cannot be used, including its known-size calibration."""
    problems = []
    if trial["returncode"] != 0:
        return [f"exit status {trial['returncode']}: {trial['stderr'].strip()}"]
    try:
        table, result = owner_rows(trial["stdout"])
    except ValueError as error:
        return [str(error)]
    if [row["phase"] for row in table] != OWNER_PHASES[trial["mode"]]:
        problems.append(f"barriers {[row['phase'] for row in table]}")
        return problems
    if result != f"result,validated={trial['history']['records']},failures=0":
        problems.append(f"result {result!r}")
    by = {row["phase"]: row for row in table}
    checks = owner_checks(trial["stdout"])
    want = {name: (value, value) for name, value in OWNER_CHECKS.items()}
    if checks != want:
        problems.append(f"known-size checks {checks}, expected {want}")
    if by["baseline"]["engine_live_bytes"] != 0:
        problems.append("the engine has bytes before it opens")
    if by["engine_dropped"]["engine_live_bytes"] != 0:
        problems.append(f"{by['engine_dropped']['engine_live_bytes']} engine bytes outlive the engine")
    for row in table:
        if row["engine_held_bytes"] != row["engine_live_bytes"] - row["engine_bytes_caller_holds"]:
            problems.append(f"{row['phase']}: held bytes do not add up")
    return problems


def spread(values):
    return peaks.spread(values)


def peak_tables(all_rows):
    out = []
    groups = {}
    for row in all_rows:
        groups.setdefault(row["history"], []).append(row)
    for name, history in groups.items():
        first = history[0]
        reps = len({row["repetition"] for row in history})
        out.append(f"### {name}\n\n{first['records']:,} records, {first['keys']:,} keys, {first['streams']:,} streams, "
                   f"{first['group']:,} records in each group, {first['payload']:,} payload bytes. "
                   f"KiB, median [range] of {reps} processes in each cell; pinned and unpinned processes together.\n\n"
                   "| Mode | Barrier | Phase that ends there | Steady RSS | Exact phase peak | Peak above both ends "
                   "| Peak above baseline | Mark low, unpinned | Mark low, pinned |\n"
                   "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |")
        for mode, phases in PHASES.items():
            for phase, work in phases.items():
                cell = [row for row in history if row["mode"] == mode and row["phase"] == phase and row["order"]]
                if not cell:
                    continue
                low = lambda pinned: spread([r["mark_low_kb"] for r in cell if r["pinned"] == pinned] or [0])
                out.append(f"| {mode} | `{phase}` | {work} | " + " | ".join(
                    spread([row[column] for row in cell]) for column in
                    ("steady_kb", "peak_kb", "transient_kb", "peak_above_baseline_kb")) +
                    f" | {low(False)} | {low(True)} |")
        out.append("")
    return "\n".join(out)


def peak_instrument(all_rows, trials):
    phases = [row for row in all_rows if row["order"]]
    lines = [f"- {len(trials)} processes, {len(all_rows)} barriers and {len(phases)} phases. "
             f"{sum(r['events'] for r in phases):,} events from the processes' own threads, "
             f"{sum(r['remote_events'] for r in phases)} from other contexts."]
    lines.append("- The replay ends at `VmRSS` at every barrier, and no instance lost an event: "
                 "both are checks that refuse a process, and every process passed them.")
    for pinned in (False, True):
        cell = [r for r in phases if r["pinned"] == pinned]
        low = [r["mark_low_kb"] for r in cell]
        missed = [value for value in low if value > 0]
        above = [-value for value in low if value < 0]
        lines.append(f"- {'Pinned' if pinned else 'Unpinned'}: the mark is below the exact peak in {len(missed)} of "
                     f"{len(cell)} phases" + (f", by {min(missed):,}–{max(missed):,} KiB (median "
                                              f"{statistics.median(missed):,})" if missed else "") +
                     f", and above it in {len(above)}" + (f", by up to {max(above):,} KiB" if above else "") + ".")
    transients = [r for r in phases if r["transient_kb"] > 0 and r["mode"] != "pulse"]
    lines.append(f"- Phases outside the pulse control that peak above both of their ends: {len(transients)} of "
                 f"{len([r for r in phases if r['mode'] != 'pulse'])}" +
                 (f", by {min(r['transient_kb'] for r in transients):,}–{max(r['transient_kb'] for r in transients):,} KiB"
                  if transients else "") + ".")
    huge = [r for r in all_rows if r["anon_huge_kb"]]
    lines.append(f"- Barriers with transparent huge pages: {len(huge)}.")
    return "\n".join(lines)


def owner_tables(owner_trials):
    groups = {}
    for trial in owner_trials:
        table, _ = owner_rows(trial["stdout"])
        for row in table:
            groups.setdefault(trial["name"], {}).setdefault((trial["mode"], row["phase"]), []).append(row)
    out = []
    kib = lambda values: spread([v / 1024 for v in values if v is not None] or [0])
    for name, cells in groups.items():
        out.append(f"### {name}\n\nKiB, median [range] of the processes in each cell. Engine bytes are requested "
                   "bytes in the engine's arena; \"held\" leaves out what the caller holds of the engine's results.\n\n"
                   "| Mode | Barrier | Engine held | Engine peak in the phase | Results the caller holds "
                   "| Caller and test live | Caller and test peak | Arena resident | Writer stack resident |\n"
                   "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
        for (mode, phase), cell in cells.items():
            out.append(f"| {mode} | `{phase}` | " + " | ".join(kib([row[c] for row in cell]) for c in (
                "engine_held_bytes", "engine_peak_bytes", "engine_bytes_caller_holds", "other_live_bytes",
                "other_peak_bytes", "arena_resident_bytes", "writer_stack_resident_bytes")) + " |")
        out.append("")
    return "\n".join(out)


def load(directory, pattern):
    return [json.loads(path.read_text()) for path in sorted(directory.glob(pattern))]


def summary(args):
    trials = load(args.directory / "peaks", "*.json")
    owners = load(args.directory / "owners", "*.json")
    label = lambda t: f"{t['name']} {t['repetition']} {t['mode']} {'pinned' if t['cpu'] is not None else 'unpinned'}"
    failed = {label(t): p for t in trials if (p := check(t))}
    failed |= {f"owner {t['name']} {t['repetition']} {t['mode']}": p for t in owners if (p := check_owner(t))}
    used = [t for t in trials if label(t) not in failed]
    matrix = [row for t in used for row in rows(t)]
    if args.csv:
        with args.csv.open("w", newline="") as output:
            writer = csv.DictWriter(output, ROW_COLUMNS, lineterminator="\n")
            writer.writeheader()
            writer.writerows(matrix)
    print(f"{len(trials)} traced processes and {len(owners)} owner processes, {len(failed)} failed.\n")
    for name, problems in failed.items():
        print(f"- FAILED {name}: {'; '.join(problems)}")
    print("## Exact resident peaks\n")
    print(peak_tables(matrix))
    print("### Instrument\n")
    print(peak_instrument(matrix, used))
    print("\n## Engine-owned bytes\n")
    print(owner_tables([t for t in owners if not check_owner(t)]))
    return bool(failed)


def parse_histories(texts):
    histories = {}
    for text in texts:
        name, _, numbers = text.partition("=")
        histories[name] = dict(zip(DIMENSIONS, map(int, numbers.split(",")), strict=True))
    return histories


def peaks_matrix(args):
    histories = parse_histories(args.history)
    output, traces = args.output / "peaks", args.traces
    output.mkdir(parents=True)
    traces.mkdir(parents=True)
    args.data.mkdir(parents=True)
    tracer = Tracer()
    tracer.open()
    try:
        def trial(name, repetition, history, mode, data, cpu):
            result = run_trial(args.binary, mode, data, history, tracer, cpu, args.timeout)
            result.update(name=name, repetition=repetition)
            pin = "pinned" if cpu is not None else "unpinned"
            stem = f"{name}-{repetition}-{pin}-{mode}"
            compact = result.pop("compact").encode()
            (traces / f"{stem}.trace.gz").write_bytes(gzip.compress(compact, mtime=0))
            result["trace_sha256"] = hashlib.sha256(compact).hexdigest()
            (output / f"{stem}.json").write_text(json.dumps(result, indent=1))
            problems = check_pulse(result) if mode == "pulse" else check(result)
            print(name, repetition, pin, mode, "; ".join(problems) or "ok", flush=True)
            if problems:
                raise SystemExit(f"stopped: {stem} failed; its data stays in {data}")

        for cpu in (None, args.cpu):
            pin = "pinned" if cpu is not None else "unpinned"
            trial("pulse", 0, PULSE, "pulse", args.data / "unused", cpu)
            for repetition in range(1, args.repetitions + 1):
                for name, history in histories.items():
                    data = args.data / f"{name}-{repetition}-{pin}"
                    for mode in ("touch", "caller", "empty", "append", "recover"):
                        # Recovery reopens the log that the append process of the same repetition wrote.
                        target = data if mode in ("append", "recover") else data.with_name(f"{data.name}-{mode}")
                        trial(name, repetition, history, mode, target, cpu)
                        if mode in ("empty", "recover"):
                            shutil.rmtree(target)
    finally:
        tracer.close()
    args.data.rmdir()


def owners_matrix(args):
    histories = parse_histories(args.history)
    output = args.output / "owners"
    output.mkdir(parents=True)
    args.data.mkdir(parents=True)
    for repetition in range(1, args.repetitions + 1):
        for name, history in histories.items():
            data = args.data / f"{name}-{repetition}"
            for mode in ("append", "recover"):
                command = [str(args.binary), mode, str(data), *(str(history[d]) for d in DIMENSIONS)]
                done = subprocess.run(command, capture_output=True, text=True, timeout=args.timeout)
                result = {"name": name, "repetition": repetition, "mode": mode, "history": history,
                          "command": command, "returncode": done.returncode, "stdout": done.stdout,
                          "stderr": done.stderr}
                (output / f"{name}-{repetition}-{mode}.json").write_text(json.dumps(result, indent=1))
                problems = check_owner(result)
                print("owner", name, repetition, mode, "; ".join(problems) or "ok", flush=True)
                if problems:
                    raise SystemExit(f"stopped: owner {name} {repetition} {mode} failed; its data stays in {data}")
            shutil.rmtree(data)
    args.data.rmdir()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    for name, function, probe in (("peaks", peaks_matrix, "resident_probe"), ("owners", owners_matrix, "owner_probe")):
        run = commands.add_parser(name, help=f"run every history under {probe}")
        run.add_argument("binary", type=Path, help=f"compiled {probe} example")
        run.add_argument("--data", type=Path, required=True, help="new directory for the logs; emptied after use")
        run.add_argument("--output", type=Path, required=True, help="directory for the readings")
        run.add_argument("--history", action="append", required=True, metavar="NAME=" + ",".join(DIMENSIONS).upper())
        run.add_argument("--repetitions", type=int, default=3)
        run.add_argument("--timeout", type=float, default=600, help="seconds before a probe is killed")
        if name == "peaks":
            run.add_argument("--traces", type=Path, required=True, help="new directory for the full event traces")
            run.add_argument("--cpu", type=int, required=True, help="processor for the pinned processes")
        run.set_defaults(function=function)
    report = commands.add_parser("summary", help="print the tables from a run's output directory")
    report.add_argument("directory", type=Path)
    report.add_argument("--csv", type=Path, help="also write every traced barrier here")
    report.set_defaults(function=summary)
    args = parser.parse_args()
    sys.exit(args.function(args))


if __name__ == "__main__":
    main()
