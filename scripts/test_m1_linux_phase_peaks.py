import importlib.util
import pathlib
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("peaks", pathlib.Path(__file__).with_name("m1-linux-phase-peaks.py"))
peaks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(peaks)

HISTORY = dict(records=64, keys=0, streams=1, group=8, payload=1024)
RESULT = "result\taccepted=0\tcommitted=0\tadmission_errors=0\tcommit_errors=0\tvalidated=0\tfailures=0"
TOUCH = ["barrier\tbaseline", "begin\ttouch\t1", "end\ttouch\t2\t700", "barrier\ttouched_held",
         "barrier\ttouched_dropped", RESULT]


def status(mark, anon, file, shmem=0, swap=0):
    return (f"Name:\tresident_probe\nVmPeak:\t  999999 kB\nVmHWM:\t{mark:8} kB\nVmRSS:\t{anon + file + shmem:8} kB\n"
            f"RssAnon:\t{anon:8} kB\nRssFile:\t{file:8} kB\nRssShmem:\t{shmem:8} kB\nVmSwap:\t{swap:8} kB\nThreads:\t2\n")


def rollup(rss, anon):
    return (f"00400000-ffffedb07000 ---p 00000000 00:00 0    [rollup]\nRss:  {rss:8} kB\nPss:  {rss:8} kB\n"
            f"Pss_Anon: {anon:8} kB\nAnonymous: {anon:8} kB\nAnonHugePages: 0 kB\nSwap: 0 kB\nSwapPss: 0 kB\n")


def reading(phase, mark, anon, file=100, reset_mark=None):
    resident = anon + file
    return {"phase": phase, "work": None, "work_ns": None, "status": status(mark, anon, file),
            "smaps_rollup": rollup(resident, anon),
            "status_after_reset": status(resident if reset_mark is None else reset_mark, anon, file)}


def trial(readings, mode="touch", history=HISTORY, name="h", repetition=1):
    return {"name": name, "repetition": repetition, "mode": mode, "history": history, "error": None,
            "returncode": 0, "stderr": "", "events": [RESULT.split("\t")], "readings": readings}


def touch_trial(held=400, mark_at_drop=380, name="h", repetition=1):
    return trial([reading("baseline", 120, 20), reading("touched_held", held + 100, held),
                  reading("touched_dropped", mark_at_drop + 100, 20)], name=name, repetition=repetition)


class ParseTest(unittest.TestCase):
    def test_status_fields_are_read_in_kib(self):
        self.assertEqual(peaks.parse_status(status(9680, 4724, 4956)),
                         {"VmHWM": 9680, "VmRSS": 9680, "RssAnon": 4724, "RssFile": 4956, "RssShmem": 0, "VmSwap": 0})

    def test_status_without_a_high_water_mark_is_refused(self):
        text = "".join(line for line in status(9680, 4724, 4956).splitlines(True) if not line.startswith("VmHWM"))
        with self.assertRaisesRegex(ValueError, "missing VmHWM"):
            peaks.parse_status(text)

    def test_status_that_is_not_in_kib_or_repeats_a_field_is_refused(self):
        good = status(9680, 4724, 4956)
        for text in (good.replace("9680 kB\nVmRSS", "9680 MB\nVmRSS"), good + "VmRSS:\t1 kB\n"):
            with self.assertRaisesRegex(ValueError, "malformed or repeated"):
                peaks.parse_status(text)

    def test_status_that_breaks_a_kernel_identity_is_refused(self):
        with self.assertRaisesRegex(ValueError, "not the sum"):
            peaks.parse_status(status(9680, 4724, 4956).replace("RssFile:\t    4956", "RssFile:\t    4000"))
        with self.assertRaisesRegex(ValueError, "below VmRSS"):
            peaks.parse_status(status(9000, 4724, 4956))

    def test_rollup_without_rss_is_refused(self):
        self.assertEqual(peaks.parse_rollup(rollup(500, 400))["Rss"], 500)
        with self.assertRaisesRegex(ValueError, "missing Rss"):
            peaks.parse_rollup(rollup(500, 400).replace("Rss:", "Rsx:"))


class RowsTest(unittest.TestCase):
    def test_steady_and_peak_come_from_the_barrier_that_ends_the_phase(self):
        baseline, held, dropped = peaks.rows(touch_trial())
        self.assertEqual((baseline["steady_kb"], baseline["peak_kb"]), (120, 120))
        self.assertEqual((held["steady_kb"], held["peak_kb"], held["anon_kb"], held["file_kb"]), (500, 500, 400, 100))
        self.assertEqual((held["steady_above_baseline_kb"], dropped["steady_above_baseline_kb"]), (380, 0))

    def test_peak_is_never_below_the_resident_set_at_the_phase_start(self):
        dropped = peaks.rows(touch_trial(held=400, mark_at_drop=380))[2]
        self.assertEqual((dropped["mark_kb"], dropped["peak_kb"], dropped["peak_above_baseline_kb"]), (480, 500, 380))
        self.assertIn("below the resident set at their start: 1 of 2; the largest difference is 20 KiB",
                      peaks.instrument(peaks.rows(touch_trial(held=400, mark_at_drop=380))))

    def test_table_gives_the_median_and_the_full_range(self):
        rows = [row for repetition, held in enumerate((400, 440, 410), 1)
                for row in peaks.rows(touch_trial(held, held, repetition=repetition))]
        table = peaks.tables(rows)
        self.assertIn("median [range] of 3 processes", table)
        self.assertIn("| touch | `baseline` | start the process | 120 | 120 | 20 | 100 | 0 | 0 |", table)
        self.assertIn("| touch | `touched_held` | allocate and fill one buffer | 510 [500–540] | 510 [500–540] "
                      "| 410 [400–440] | 100 | 390 [380–420] | 390 [380–420] |", table)

    def test_an_even_count_keeps_a_fractional_median(self):
        self.assertEqual(peaks.spread([1, 2, 4, 9]), "3 [1–9]")
        self.assertEqual(peaks.spread([1, 2, 3, 9]), "2.5 [1–9]")
        self.assertEqual(peaks.spread([7, 7]), "7")


class CheckTest(unittest.TestCase):
    def test_a_complete_trial_is_usable(self):
        self.assertEqual(peaks.check(touch_trial()), [])

    def test_a_missing_barrier_a_bad_result_and_an_unreadable_status_are_refused(self):
        missing = touch_trial()
        del missing["readings"][1]
        self.assertIn("expected ['baseline', 'touched_held', 'touched_dropped']", peaks.check(missing)[0])
        failed = touch_trial()
        failed["events"] = [RESULT.replace("failures=0", "failures=1").split("\t")]
        failed["returncode"] = 1
        self.assertEqual(len(peaks.check(failed)), 2)
        unread = touch_trial()
        unread["readings"][2]["status"] = "Name:\tresident_probe\n"
        self.assertIn("touched_dropped: status: missing VmHWM", peaks.check(unread)[0])
        short = trial(touch_trial()["readings"], mode="append", history=dict(HISTORY, records=64))
        self.assertTrue(any("'validated': 64" in problem for problem in peaks.check(short)))

    def test_reset_check_accepts_a_reset_and_refuses_one_that_did_nothing(self):
        history = peaks.RESET_CHECK
        size = history["records"] * history["payload"] // 1024

        def run(reset_mark, dropped_anon=20, mark=None):
            return trial([reading("baseline", 120, 20), reading("touched_held", size + 124, size + 24),
                          reading("touched_dropped", mark or size + 124, dropped_anon, reset_mark=reset_mark)],
                         history=history)

        self.assertEqual(peaks.check_reset(run(reset_mark=120)), [])
        self.assertEqual(peaks.check_reset(run(reset_mark=140)), [])
        self.assertIn("after the reset the mark is", peaks.check_reset(run(reset_mark=size + 124))[0])
        self.assertIn("stayed resident", peaks.check_reset(run(reset_mark=size + 124, dropped_anon=size + 24))[0])
        self.assertIn("missed the held", peaks.check_reset(run(reset_mark=120, mark=size // 2))[0])


class DriverTest(unittest.TestCase):
    def run_fake(self, lines, status_text, code=0):
        with tempfile.TemporaryDirectory() as temp:
            temp = pathlib.Path(temp)
            probe = temp / "probe"
            probe.write_text(f"#!{sys.executable}\nimport sys\nfor line in {lines!r}:\n    print(line, flush=True)\n"
                             "    if line.startswith('barrier') and sys.stdin.readline() != 'go\\n':\n"
                             f"        sys.exit(3)\nsys.exit({code})\n")
            probe.chmod(0o755)
            (temp / "status").write_text(status_text)
            (temp / "smaps_rollup").write_text(rollup(220, 120))
            result = peaks.run_trial(probe, "touch", temp / "data", HISTORY, timeout=30, proc=lambda pid: temp)
            return result, (temp / "clear_refs").read_text() if (temp / "clear_refs").exists() else None

    def test_driver_reads_and_resets_at_each_barrier_before_it_releases_the_probe(self):
        result, reset = self.run_fake(TOUCH, status(220, 120, 100))
        self.assertEqual(reset, "5")
        self.assertEqual(peaks.check(result), [])
        self.assertEqual([(r["phase"], r["work"], r["work_ns"]) for r in result["readings"]],
                         [("baseline", None, None), ("touched_held", "touch", 700), ("touched_dropped", None, None)])
        self.assertEqual(result["command"][1:], ["touch", result["command"][2], "64", "0", "1", "8", "1024"])
        self.assertEqual([row["steady_kb"] for row in peaks.rows(result)], [220, 220, 220])

    def test_driver_keeps_the_raw_text_of_a_status_it_cannot_use(self):
        result, _ = self.run_fake(TOUCH, "Name:\tresident_probe\nVmRSS:\t 220 kB\n")
        self.assertEqual(len(result["readings"]), 3)
        self.assertEqual(result["readings"][0]["status"], "Name:\tresident_probe\nVmRSS:\t 220 kB\n")
        self.assertEqual(len(peaks.check(result)), 3)

    def test_driver_reports_a_probe_that_fails_or_stops_early(self):
        result, _ = self.run_fake(TOUCH[:2], status(220, 120, 100), code=1)
        problems = peaks.check(result)
        self.assertEqual(result["returncode"], 1)
        self.assertEqual(len(problems), 3, problems)


if __name__ == "__main__":
    unittest.main()
