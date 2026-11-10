import importlib.util
import io
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

SCRIPTS = Path(__file__).parent


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


summarize = load("m1-summarize")
quiet = load("m1-quiet")


def log(records_per_second, p99, device_p99, paced=False, errors=0):
    lines = [
        "CONFIG callers=1000 rounds=1000 payload=256 linger_ns=1000000 group_byte_cap=1048576",
        f"METRIC device_fdatasync_ns count=1000 min=1 p50=300000 p99={device_p99} p999=1 max=1 sum=1",
        f"APPEND records=1000000 elapsed_ns=1 records_per_second={records_per_second} full=0 "
        f"other_admission_errors=0 commit_errors={errors} correctness_errors=0",
        f"METRIC append_ns count=1000000 min=1 p50=1200000 p99={p99} p999=5000000 max=9000000 sum=1300000000000",
        "METRIC group_submits count=20000 min=1 p50=50 p99=60 p999=70 max=80 sum=1000000",
        "STORAGE kind=Group.SyncData count=20000 sum_ns=8000000000",
        "REOPEN ns=500000000 start_records=1000000 end_records=1000000 end_bytes=296000000 correctness_errors=0",
        "test result: ok. 1 passed; 0 failed",
    ]
    if paced:
        lines.insert(0, "PACED interval_ns=20000000 offered_records_per_second=50000.000 latency_from=scheduled_send")
    return "\n".join(lines) + "\n"


class Summarize(unittest.TestCase):
    def capture(self, root):
        out = io.StringIO()
        with redirect_stdout(out):
            summarize.main(root)
        return out.getvalue()

    def write(self, root, paced_p99=(1_900_000,) * 5, errors=0):
        for n in range(1, 6):
            (root / f"run-{n}.log").write_text(log("400000.000", 4_000_000, 450_000, errors=errors))
        for n, p99 in enumerate(paced_p99, 1):
            (root / f"paced-{n}.log").write_text(log("50000.000", p99, 450_000, paced=True))

    def test_the_bound_is_linger_plus_twice_the_device_p99_of_the_same_run(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.write(root, paced_p99=(1_900_000, 1_900_000, 1_900_001, 2_500_000, 1_000_000))
            text = self.capture(root)
        self.assertIn("| 1.900 | +0.000 |", text)
        self.assertIn("| 1.900 | +0.600 |", text)
        self.assertIn("within the bound of the same run: 3 of 5", text)
        self.assertIn("| 1 | 1.300 | 2.500 | 20000 | 400 |", text)

    def test_default_runs_alone_give_no_paced_table(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.write(root, paced_p99=())
            text = self.capture(root)
        self.assertNotIn("paced", text.lower())

    def test_a_run_with_a_commit_error_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.write(root, errors=1)
            with self.assertRaises(AssertionError):
                self.capture(root)

    def test_a_closed_loop_log_among_the_paced_runs_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.write(root)
            (root / "paced-3.log").write_text(log("50000.000", 1_900_000, 450_000))
            with self.assertRaises(AssertionError):
                self.capture(root)


class Quiet(unittest.TestCase):
    def test_busy_share_counts_everything_that_is_not_idle(self):
        self.assertAlmostEqual(quiet.busy_percent((1000, 900), (2000, 1800)), 10.0)
        self.assertAlmostEqual(quiet.busy_percent((1000, 900), (2000, 1900)), 0.0)
        self.assertEqual(quiet.busy_percent((1000, 900), (1000, 900)), 0.0)


if __name__ == "__main__":
    unittest.main()
