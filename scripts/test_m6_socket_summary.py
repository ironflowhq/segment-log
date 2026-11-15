import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("m6_socket_summary", Path(__file__).with_name("m6-socket-summary.py"))
summary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(summary)

RUN = "CONFIG transport=x\nAPPEND records=1000 elapsed_ns=1 records_per_second={rate} full=0\nMETRIC append_ns count=1000 min=1 p50={p50} p99=4000000 p999=5000000 max=6\n"


def capture(drop=None):
    root = Path(tempfile.mkdtemp())
    for mode in ["closed", "paced"]:
        (root / mode).mkdir()
        (root / mode / "warmup-unix.log").write_text(RUN.format(rate=1, p50=1))
        for pair, (inproc, unix) in enumerate([(1000, 800), (1200, 900), (1100, 1000)], 1):
            (root / mode / f"inproc-{pair}.log").write_text(RUN.format(rate=inproc, p50=2000000))
            (root / mode / f"unix-{pair}.log").write_text(RUN.format(rate=unix, p50=3000000))
    if drop:
        (root / drop).unlink()
    return root


class SummaryTests(unittest.TestCase):
    def test_medians_ranges_and_change_skip_warmups(self):
        text = summary.summarize(capture())
        self.assertIn("| records/s | 1,100 (1,000 to 1,200) | 900 (800 to 1,000) | -18.2% |", text)
        self.assertIn("| p50 ms | 2.000 (2.000 to 2.000) | 3.000 (3.000 to 3.000) | +50.0% |", text)
        self.assertIn("## paced (3 pairs", text)

    def test_an_unpaired_run_fails(self):
        with self.assertRaises(SystemExit):
            summary.summarize(capture(drop="paced/unix-2.log"))

    def test_a_truncated_run_fails(self):
        root = capture()
        (root / "closed" / "unix-1.log").write_text("CONFIG transport=unix\n")
        with self.assertRaises(SystemExit):
            summary.summarize(root)


if __name__ == "__main__":
    unittest.main()
