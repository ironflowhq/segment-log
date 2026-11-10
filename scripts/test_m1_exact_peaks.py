import importlib.util
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("exact", pathlib.Path(__file__).with_name("m1-exact-peaks.py"))
exact = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exact)

MM = 3311
RESULT = "result\taccepted=0\tcommitted=0\tadmission_errors=0\tcommit_errors=0\tvalidated=0\tfailures=0"
PULSE = dict(records=64, keys=0, streams=1, group=1, payload=1024)


def line(member, kib, stamp=1.0, mm=MM, curr=1, cpu=3):
    return (f"   resident_probe-4242    [{cpu:03}] ..... {stamp:.6f}: rss_stat: mm_id={mm} curr={curr} "
            f"type={member} size={kib * 1024}B\n")


def status(anon, file=100, mark=None, swap=0, cpus="0-9"):
    resident = anon + file
    return (f"Name:\tresident_probe\nVmHWM:\t{mark or resident:8} kB\nVmRSS:\t{resident:8} kB\n"
            f"RssAnon:\t{anon:8} kB\nRssFile:\t{file:8} kB\nRssShmem:\t       0 kB\nVmSwap:\t{swap:8} kB\n"
            f"Cpus_allowed_list:\t{cpus}\n")


def rollup(rss, anon):
    return (f"Rss:  {rss:8} kB\nPss:  {rss:8} kB\nAnonymous: {anon:8} kB\nAnonHugePages: 0 kB\nSwap: 0 kB\n")


def members(anon, file=100):
    return {"MM_FILEPAGES": file * 1024, "MM_ANONPAGES": anon * 1024, "MM_SHMEMPAGES": 0, "MM_SWAPENTS": 0}


class ParseTest(unittest.TestCase):
    def test_an_event_line_gives_processor_time_memory_and_exact_bytes(self):
        self.assertEqual(exact.parse_events(line("MM_ANONPAGES", 120, 50581.532407)),
                         [(3, 50581.532407, MM, 1, "MM_ANONPAGES", 122880)])

    def test_a_lost_events_marker_or_any_other_line_is_refused(self):
        for text in ("CPU:3 [LOST 17 EVENTS]\n", line("MM_ANONPAGES", 1).replace("rss_stat", "mm_page_alloc")):
            with self.assertRaisesRegex(ValueError, "unexpected line"):
                exact.parse_events(text)


class ReplayTest(unittest.TestCase):
    def test_the_peak_is_the_largest_resident_sum_between_events(self):
        events = exact.parse_events(line("MM_ANONPAGES", 500) + line("MM_FILEPAGES", 140) + line("MM_ANONPAGES", 30))
        end, peak, at = exact.replay(members(20), events)
        self.assertEqual((end["MM_ANONPAGES"], end["MM_FILEPAGES"]), (30 * 1024, 140 * 1024))
        self.assertEqual((peak, at), ((500 + 140) * 1024, 1))

    def test_a_phase_never_peaks_below_its_start(self):
        _, peak, at = exact.replay(members(400), exact.parse_events(line("MM_ANONPAGES", 30)))
        self.assertEqual((peak, at), (500 * 1024, None))

    def test_swap_entries_are_not_resident(self):
        _, peak, _ = exact.replay(members(20), exact.parse_events(line("MM_SWAPENTS", 9000)))
        self.assertEqual(peak, 120 * 1024)


class PhaseTraceTest(unittest.TestCase):
    def test_the_first_event_names_the_memory_and_remote_events_merge_in_time_order(self):
        own = line("MM_ANONPAGES", 300, 2.0) + line("MM_ANONPAGES", 20, 4.0)
        remote = line("MM_ANONPAGES", 900, 3.0, curr=0) + line("MM_ANONPAGES", 5000, 3.5, mm=1, curr=0)
        traced, mm = exact.phase_trace(members(20), own, remote, None)
        self.assertEqual(mm, MM)
        self.assertEqual(traced["peak_bytes"], 1000 * 1024)
        self.assertEqual(len(traced["remote_events"]), 1)
        self.assertEqual(traced["end"]["MM_ANONPAGES"], 20 * 1024)

    def test_a_thread_that_changes_another_memory_is_refused(self):
        with self.assertRaisesRegex(ValueError, "more than one memory"):
            exact.phase_trace(members(20), line("MM_ANONPAGES", 30) + line("MM_ANONPAGES", 40, mm=9), "", None)
        with self.assertRaisesRegex(ValueError, "not its own"):
            exact.phase_trace(members(20), line("MM_ANONPAGES", 30, curr=0), "", MM)


class FakeTracer:
    """Hands out queued drains, as trace_pipe would, and records what the driver asked of it."""

    def __init__(self, drains=(), lost=0):
        self.drains, self.lost, self.started, self.stopped = list(drains), lost, [], 0

    def start(self, pid):
        self.started.append(pid)

    def drain(self):
        return self.drains.pop(0) if self.drains else ("", "")

    def losses(self):
        return {"own": self.lost, "remote": 0}

    def stop(self):
        self.stopped += 1


class SnapshotTest(unittest.TestCase):
    def test_status_is_read_again_until_no_event_arrives_around_it(self):
        with tempfile.TemporaryDirectory() as directory:
            proc = pathlib.Path(directory)
            (proc / "status").write_text(status(20))
            tracer = FakeTracer([(line("MM_ANONPAGES", 30), ""), (line("MM_ANONPAGES", 40), ""), ("", "")])
            events = {"own": "", "remote": ""}
            exact.snapshot(proc, tracer, events)
            self.assertEqual(events["own"].count("rss_stat"), 2)
            self.assertEqual(tracer.drains, [])

    def test_counters_that_never_settle_are_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            proc = pathlib.Path(directory)
            (proc / "status").write_text(status(20))
            tracer = FakeTracer([(line("MM_ANONPAGES", n), "") for n in range(exact.SNAPSHOT_TRIES + 1)])
            with mock.patch.object(exact.time, "sleep"), self.assertRaisesRegex(ValueError, "did not settle"):
                exact.snapshot(proc, tracer, {"own": "", "remote": ""})


def reading(phase, anon, events=None, mark=None, cpus="0-9", start=None):
    traced = None
    if events is not None:
        traced, _ = exact.phase_trace(start, events, "", MM)
        traced.pop("compact")
    return {"phase": phase, "work": None, "work_ns": None, "status": status(anon, mark=mark, cpus=cpus),
            "cpus_allowed": cpus, "smaps_rollup": rollup(anon + 100, anon),
            "status_after_reset": status(anon), "trace": traced}


def pulse_trial(transient_kib, end_kib=20, cpu=None):
    events = line("MM_ANONPAGES", 20 + transient_kib) + line("MM_ANONPAGES", end_kib, 2.0)
    readings = [reading("baseline", 20, cpus=str(cpu) if cpu is not None else "0-9"),
                reading("pulsed", end_kib, events, mark=end_kib + 100, start=members(20))]
    return {"name": "pulse", "repetition": 0, "mode": "pulse", "history": PULSE, "cpu": cpu, "error": None,
            "returncode": 0, "stderr": "", "events": [RESULT.split("\t")], "readings": readings,
            "losses": {"own": 0, "remote": 0}}


class CheckTest(unittest.TestCase):
    def test_a_pulse_of_its_buffer_size_passes_and_the_mark_error_is_reported(self):
        trial = pulse_trial(64 + 4)
        self.assertEqual(exact.check_pulse(trial), [])
        baseline, pulsed = exact.rows(trial)
        self.assertEqual((pulsed["transient_kb"], pulsed["mark_low_kb"]), (68, 68))

    def test_a_pulse_that_the_replay_misses_is_refused(self):
        self.assertIn("replayed as a 32 KiB transient", "; ".join(exact.check_pulse(pulse_trial(32))))

    def test_a_replay_that_ends_away_from_vmrss_is_refused(self):
        trial = pulse_trial(68)
        trial["readings"][1]["status"] = status(24)
        self.assertIn("the replay ends at", "; ".join(exact.check(trial)))

    def test_lost_events_and_a_pin_that_did_not_hold_are_refused(self):
        trial = pulse_trial(68, cpu=7)
        trial["losses"] = {"own": 3, "remote": 0}
        trial["readings"][0]["cpus_allowed"] = "0-9"
        problems = "; ".join(exact.check(trial))
        self.assertIn("lost events", problems)
        self.assertIn("pinned to 0-9, expected 7", problems)


class RunTrialTest(unittest.TestCase):
    def test_a_stand_in_probe_is_traced_through_every_barrier(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            probe = root / "probe.py"
            probe.write_text("import sys\nfor phase in ('baseline', 'pulsed'):\n"
                             "    print('barrier\\t' + phase, flush=True)\n"
                             "    assert sys.stdin.readline().strip() == 'go'\n"
                             f"print({RESULT!r})\n")
            probe.chmod(0o755)
            proc = root / "proc"
            proc.mkdir()
            (proc / "status").write_text(status(20))
            (proc / "smaps_rollup").write_text(rollup(120, 20))
            (proc / "clear_refs").write_text("")
            tracer = FakeTracer([(line("MM_ANONPAGES", 88) + line("MM_ANONPAGES", 20, 2.0), ""), ("", "")])
            binary = pathlib.Path(sys.executable)
            trial = exact.run_trial(binary, str(probe), root / "data", PULSE, tracer, proc=lambda pid: proc)
            # The stand-in takes the mode argument as its script; drop it from the barrier check.
            trial["mode"] = "pulse"
            self.assertEqual(exact.check_pulse(trial), [])
            self.assertEqual(len(tracer.started), 1)
            self.assertEqual(tracer.stopped, 1)
            self.assertEqual((proc / "clear_refs").read_text(), "5")
            self.assertIn("# pulsed", trial["compact"])


def owner_output(engine_after_drop=0, held_error=0, page_check=1 << 20):
    rows = [("baseline", 0, 0, 0, 0)]
    phases = ["empty_held", "caller_held", "appended_held", "read_one_held", "read_one_dropped",
              "validation_held", "validation_dropped"]
    rows += [(phase, 9000, 9500, 400 if phase.endswith("held") and "read" in phase else 0, 0) for phase in phases]
    rows.append(("engine_dropped", engine_after_drop, 9000, 0, 0))
    lines = ["phase," + ",".join(exact.OWNER_COLUMNS)]
    for phase, live, peak, holds, _ in rows:
        held = live - holds + (held_error if phase == "appended_held" else 0)
        lines.append(f"{phase},{live},{peak},{holds},{held},1833,1833,8192,{'-' if phase == 'baseline' else 12288}")
    checks = dict(exact.OWNER_CHECKS, pages_touched=page_check)
    lines += [f"check,{name},{exact.OWNER_CHECKS[name]},{value}" for name, value in checks.items()]
    lines.append("result,validated=64,failures=0")
    return "\n".join(lines) + "\n"


def owner_trial(text):
    return {"name": "h", "repetition": 1, "mode": "append", "history": dict(PULSE, payload=32), "returncode": 0,
            "stdout": text, "stderr": ""}


class OwnerTest(unittest.TestCase):
    def test_exact_known_size_checks_and_consistent_rows_pass(self):
        self.assertEqual(exact.check_owner(owner_trial(owner_output())), [])
        table, _ = exact.owner_rows(owner_output())
        self.assertIsNone(table[0]["writer_stack_resident_bytes"])

    def test_a_page_count_that_is_not_exact_is_refused(self):
        self.assertIn("known-size checks", "; ".join(exact.check_owner(owner_trial(owner_output(page_check=1 << 19)))))

    def test_engine_bytes_that_outlive_the_engine_are_refused(self):
        self.assertIn("outlive the engine", "; ".join(exact.check_owner(owner_trial(owner_output(engine_after_drop=8)))))

    def test_held_bytes_that_do_not_add_up_are_refused(self):
        self.assertIn("do not add up", "; ".join(exact.check_owner(owner_trial(owner_output(held_error=1)))))


if __name__ == "__main__":
    unittest.main()
