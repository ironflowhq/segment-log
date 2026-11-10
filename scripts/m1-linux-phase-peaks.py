#!/usr/bin/env python3
"""Read the kernel's resident-set counters at each barrier of resident_probe on Linux.

The probe stops at a barrier and waits. The driver reads /proc/<pid>/status and
/proc/<pid>/smaps_rollup, resets the high-water mark through /proc/<pid>/clear_refs, and
releases the probe. Nothing is sampled: VmRSS is the resident set at the barrier, and VmHWM is
the kernel's mark for the phase that ended there.
"""
import argparse
import csv
import json
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys
import threading

STATUS_FIELDS = ("VmHWM", "VmRSS", "RssAnon", "RssFile", "RssShmem", "VmSwap")
ROLLUP_FIELDS = ("Rss", "Pss", "Anonymous", "AnonHugePages", "Swap")
DIMENSIONS = ("records", "keys", "streams", "group", "payload")
START = "start the process"
# Each mode's barriers in the probe's order, with the work that ends at each barrier.
PHASES = {
    "touch": {"baseline": START, "touched_held": "allocate and fill one buffer",
              "touched_dropped": "check and drop the buffer"},
    "caller": {"baseline": START, "caller_held": "build every submit",
               "caller_dropped": "drop every submit"},
    "empty": {"baseline": START, "empty_held": "open a new log",
              "validation_held": "read the empty log", "validation_dropped": "drop the empty result",
              "engine_dropped": "drop the engine"},
    "append": {"baseline": START, "empty_held": "open a new log", "appended_held": "append every group",
               "read_one_held": "read one record", "read_one_dropped": "check and drop one record",
               "validation_held": "read every record", "validation_dropped": "check and drop every record",
               "engine_dropped": "drop the engine"},
    "recover": {"baseline": START, "recovered_held": "reopen the log",
                "read_one_held": "read one record", "read_one_dropped": "check and drop one record",
                "validation_held": "read every record", "validation_dropped": "check and drop every record",
                "engine_dropped": "drop the engine"},
}
# The reset check touches a known 64 MiB. Its bounds are loose against that size on purpose:
# they catch a reset that did nothing, and the exact differences are reported.
RESET_CHECK = dict(records=65536, keys=0, streams=1, group=1, payload=1024)
RESET_CHECK_SLACK_KB = 4096
COLUMNS = ("history", *DIMENSIONS, "mode", "repetition", "order", "phase", "work", "work_ns",
           "steady_kb", "mark_kb", "peak_kb", "anon_kb", "file_kb", "shmem_kb", "swap_kb",
           "rollup_rss_kb", "rollup_anon_kb", "rollup_pss_kb", "anon_huge_kb",
           "reset_mark_kb", "reset_rss_kb", "steady_above_baseline_kb", "peak_above_baseline_kb")


def kilobytes(text, names, source):
    """Return the named `Name: N kB` fields. A missing field is an error, never a zero."""
    found = {}
    for line in text.splitlines():
        name, _, rest = line.partition(":")
        if name in names:
            match = re.fullmatch(r"\s*(\d+) kB", rest)
            if not match or name in found:
                raise ValueError(f"{source}: malformed or repeated {name}")
            found[name] = int(match[1])
    missing = [name for name in names if name not in found]
    if missing:
        raise ValueError(f"{source}: missing {', '.join(missing)}")
    return found


def parse_status(text):
    status = kilobytes(text, STATUS_FIELDS, "status")
    # The kernel prints both identities from one set of counters. A text that breaks one is not
    # a status file this script understands.
    if status["VmRSS"] != status["RssAnon"] + status["RssFile"] + status["RssShmem"]:
        raise ValueError("status: VmRSS is not the sum of RssAnon, RssFile and RssShmem")
    if status["VmHWM"] < status["VmRSS"]:
        raise ValueError("status: VmHWM is below VmRSS")
    return status


def parse_rollup(text):
    return kilobytes(text, ROLLUP_FIELDS, "smaps_rollup")


def read_barrier(proc):
    """Read the counters of a process that waits at a barrier, then reset its high-water mark."""
    status = (proc / "status").read_text()
    rollup = (proc / "smaps_rollup").read_text()
    # 5 resets only the peak resident set. Every other value clears page reference bits.
    (proc / "clear_refs").write_text("5")
    return {"status": status, "smaps_rollup": rollup, "status_after_reset": (proc / "status").read_text()}


def run_trial(binary, mode, data, history, timeout=300, proc=lambda pid: Path(f"/proc/{pid}")):
    """Run one probe process. Keep every line and every raw reading, also when it fails."""
    command = [str(binary), mode, str(data), *(str(history[name]) for name in DIMENSIONS)]
    trial = {"command": command, "mode": mode, "history": history, "events": [], "readings": [],
             "error": None}
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True)
    watchdog = threading.Timer(timeout, process.kill)
    watchdog.start()
    work = None
    try:
        for line in process.stdout:
            parts = line.rstrip("\n").split("\t")
            trial["events"].append(parts)
            if parts[0] == "end":
                work = parts
            elif parts[0] == "barrier":
                reading = read_barrier(proc(process.pid))
                trial["readings"].append({"phase": parts[1], "work": work and work[1],
                                          "work_ns": work and int(work[3]), **reading})
                work = None
                process.stdin.write("go\n")
                process.stdin.flush()
    except (OSError, ValueError, IndexError) as error:
        trial["error"] = repr(error)
        process.kill()
    finally:
        watchdog.cancel()
    trial["stderr"] = process.stderr.read()
    trial["returncode"] = process.wait()
    for pipe in (process.stdin, process.stdout, process.stderr):
        pipe.close()
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
    for reading in trial["readings"]:
        try:
            parse_status(reading["status"])
            parse_status(reading["status_after_reset"])
            parse_rollup(reading["smaps_rollup"])
        except ValueError as error:
            problems.append(f"{reading['phase']}: {error}")
    return problems


def rows(trial):
    """Reduce one usable trial to one row for each barrier. All sizes are in KiB, as the kernel prints them."""
    out = []
    for order, reading in enumerate(trial["readings"]):
        status = parse_status(reading["status"])
        rollup = parse_rollup(reading["smaps_rollup"])
        reset = parse_status(reading["status_after_reset"])
        # The resident set at the start of a phase is known exactly, and the phase cannot peak
        # below it. The kernel's mark can read lower, because it comes from per-CPU counters.
        peak = max(status["VmHWM"], out[-1]["steady_kb"]) if out else status["VmHWM"]
        baseline = out[0]["steady_kb"] if out else status["VmRSS"]
        out.append({"history": trial.get("name", ""), **trial["history"], "mode": trial["mode"],
                    "repetition": trial.get("repetition", 0), "order": order, "phase": reading["phase"],
                    "work": reading["work"] or "", "work_ns": reading["work_ns"] or "",
                    "steady_kb": status["VmRSS"], "mark_kb": status["VmHWM"], "peak_kb": peak,
                    "anon_kb": status["RssAnon"], "file_kb": status["RssFile"],
                    "shmem_kb": status["RssShmem"], "swap_kb": status["VmSwap"] + rollup["Swap"],
                    "rollup_rss_kb": rollup["Rss"], "rollup_anon_kb": rollup["Anonymous"],
                    "rollup_pss_kb": rollup["Pss"], "anon_huge_kb": rollup["AnonHugePages"],
                    "reset_mark_kb": reset["VmHWM"], "reset_rss_kb": reset["VmRSS"],
                    "steady_above_baseline_kb": status["VmRSS"] - baseline,
                    "peak_above_baseline_kb": peak - baseline})
    return out


def check_reset(trial):
    """Return why the kernel's reset cannot be trusted, from one touch trial of a known size."""
    problems = check(trial)
    if problems:
        return problems
    baseline, held, dropped = rows(trial)
    size = trial["history"]["records"] * trial["history"]["payload"] // 1024
    grown = held["steady_kb"] - baseline["steady_kb"]
    if not size <= grown <= size + RESET_CHECK_SLACK_KB:
        problems.append(f"touching {size} KiB grew the resident set by {grown} KiB")
    if dropped["mark_kb"] < held["steady_kb"] - RESET_CHECK_SLACK_KB:
        problems.append(f"the mark {dropped['mark_kb']} KiB missed the held {held['steady_kb']} KiB")
    if dropped["steady_kb"] > held["steady_kb"] - size // 2:
        problems.append("the buffer stayed resident after its drop, so the reset has nothing to show")
    elif dropped["reset_mark_kb"] > dropped["reset_rss_kb"] + RESET_CHECK_SLACK_KB:
        problems.append(f"after the reset the mark is {dropped['reset_mark_kb']} KiB "
                        f"with {dropped['reset_rss_kb']} KiB resident")
    return problems


def spread(values):
    """Median and full range. One value alone means every repetition gave it."""
    number = lambda value: f"{value:,.1f}" if value % 1 else f"{int(value):,}"
    low, high = min(values), max(values)
    return number(low) if low == high else f"{number(statistics.median(values))} [{number(low)}–{number(high)}]"


def tables(all_rows):
    """One Markdown table for each history: every mode's barriers, engine phases beside the controls."""
    out = []
    histories = {}
    for row in all_rows:
        histories.setdefault(row["history"], []).append(row)
    for name, history in histories.items():
        first = history[0]
        repetitions = len({row["repetition"] for row in history})
        out.append(f"### {name}\n\n{first['records']:,} records, {first['keys']:,} keys, {first['streams']:,} streams, "
                   f"{first['group']:,} records in each group, {first['payload']:,} payload bytes. "
                   f"KiB, median [range] of {repetitions} processes for each mode.\n\n"
                   "| Mode | Barrier | Phase that ends there | Steady RSS | Phase peak | Anonymous | File-backed "
                   "| Steady above baseline | Peak above baseline |\n"
                   "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |")
        for mode, phases in PHASES.items():
            for phase, work in phases.items():
                cell = [row for row in history if row["mode"] == mode and row["phase"] == phase]
                if cell:
                    out.append(f"| {mode} | `{phase}` | {work} | " + " | ".join(
                        spread([row[column] for row in cell]) for column in
                        ("steady_kb", "peak_kb", "anon_kb", "file_kb", "steady_above_baseline_kb",
                         "peak_above_baseline_kb")) + " |")
        out.append("")
    return "\n".join(out)


def instrument(all_rows):
    """What the readings say about the instrument itself, so that a reader can trust or doubt it."""
    lines = [f"- {len(all_rows)} barriers in {len({(r['history'], r['mode'], r['repetition']) for r in all_rows})} processes, "
             "the reset check included."]

    def count(label, differences):
        nonzero = [abs(d) for d in differences if d]
        largest = f"; the largest difference is {max(nonzero):,} KiB" if nonzero else ""
        lines.append(f"- {label}: {len(nonzero)} of {len(differences)}{largest}.")

    count("`VmRSS` differs from the `Rss` of `smaps_rollup`", [r["steady_kb"] - r["rollup_rss_kb"] for r in all_rows])
    count("`RssAnon` differs from the `Anonymous` of `smaps_rollup`", [r["anon_kb"] - r["rollup_anon_kb"] for r in all_rows])
    count("Barriers with swapped pages", [r["swap_kb"] for r in all_rows])
    count("Barriers with shared-memory pages", [r["shmem_kb"] for r in all_rows])
    count("Barriers with transparent huge pages", [r["anon_huge_kb"] for r in all_rows])
    count("Resets that changed `VmRSS`", [r["reset_rss_kb"] - r["steady_kb"] for r in all_rows])
    count("Resets that left the mark above `VmRSS`", [r["reset_mark_kb"] - r["reset_rss_kb"] for r in all_rows])
    # Rows of one process are adjacent, and a nonzero order means the row before is the same process.
    count("Phases whose mark is below the resident set at their start",
          [max(0, before["steady_kb"] - row["mark_kb"]) for before, row in zip(all_rows, all_rows[1:]) if row["order"]])
    count("Touched buffers whose resident growth differs from their size",
          [r["steady_above_baseline_kb"] - r["records"] * r["payload"] // 1024
           for r in all_rows if r["phase"] == "touched_held"])
    return "\n".join(lines)


def load(directory):
    trials = [json.loads(path.read_text()) for path in sorted(directory.glob("*.json"))]
    if not trials:
        raise SystemExit(f"no trials in {directory}")
    return trials


def summary(args):
    trials = load(args.directory)
    label = lambda trial: f"{trial['name']} {trial['repetition']} {trial['mode']}"
    failed = {label(trial): problems for trial in trials if (problems := check(trial))}
    used = [trial for trial in trials if label(trial) not in failed]
    matrix = [row for trial in used if trial["repetition"] for row in rows(trial)]
    checks = [row for trial in used if not trial["repetition"] for row in rows(trial)]
    if args.csv:
        with args.csv.open("w", newline="") as output:
            writer = csv.DictWriter(output, COLUMNS, lineterminator="\n")
            writer.writeheader()
            writer.writerows(checks + matrix)
    print(f"{len(trials)} processes, {len(failed)} failed.\n")
    for name, problems in failed.items():
        print(f"- FAILED {name}: {'; '.join(problems)}")
    print(tables(matrix))
    print("### Instrument\n")
    print(instrument(checks + matrix))
    return bool(failed)


def matrix(args):
    histories = {}
    for text in args.history:
        name, _, numbers = text.partition("=")
        histories[name] = dict(zip(DIMENSIONS, map(int, numbers.split(",")), strict=True))
    # Both directories must be new, so a run never reads another run's log or overwrites its readings.
    args.output.mkdir(parents=True)
    args.data.mkdir(parents=True)

    def trial(name, repetition, history, mode, data):
        result = run_trial(args.binary, mode, data, history, args.timeout)
        result.update(name=name, repetition=repetition)
        (args.output / f"{name}-{repetition}-{mode}.json").write_text(json.dumps(result, indent=1))
        problems = check(result)
        print(name, repetition, mode, "; ".join(problems) or "ok", flush=True)
        if problems:
            raise SystemExit(f"stopped: {name} {repetition} {mode} failed; its data stays in {data}")
        return result

    problems = check_reset(trial("reset-check", 0, RESET_CHECK, "touch", args.data / "unused"))
    if problems:
        raise SystemExit("stopped: the high-water mark reset failed its check: " + "; ".join(problems))
    for repetition in range(1, args.repetitions + 1):
        for name, history in histories.items():
            data = args.data / f"{name}-{repetition}"
            for mode in PHASES:
                # Recovery reopens the log that the append process of the same repetition wrote.
                target = data if mode in ("append", "recover") else data.with_name(f"{data.name}-{mode}")
                trial(name, repetition, history, mode, target)
                if mode in ("empty", "recover"):
                    shutil.rmtree(target)
    args.data.rmdir()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("matrix", help="check the reset, then run every mode of every history")
    run.add_argument("binary", type=Path, help="compiled resident_probe example")
    run.add_argument("--data", type=Path, required=True, help="new directory for the logs; emptied after use")
    run.add_argument("--output", type=Path, required=True, help="new directory for one JSON file per process")
    run.add_argument("--history", action="append", required=True, metavar="NAME=" + ",".join(DIMENSIONS).upper())
    run.add_argument("--repetitions", type=int, default=3)
    run.add_argument("--timeout", type=float, default=300, help="seconds before a probe is killed")
    run.set_defaults(function=matrix)
    report = commands.add_parser("summary", help="print one table per history from a matrix output directory")
    report.add_argument("directory", type=Path)
    report.add_argument("--csv", type=Path, help="also write every barrier's reading here")
    report.set_defaults(function=summary)
    args = parser.parse_args()
    sys.exit(args.function(args))


if __name__ == "__main__":
    main()
