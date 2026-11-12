import contextlib
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("m4_summary", Path(__file__).with_name("m4-summary.py"))
m4_summary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m4_summary)

FIXTURE = "BUILD ns=1000000000\nFIXTURE records=13000 sealed_segments=3 min_records_per_sealed=4000 max_records_per_sealed=4000 record_entry_bytes=296 active_bytes=296096 active_records=1000 active_keys={keys} dir_bytes=4178096\nFILTERS sealed_keys={sealed} expected_filter_bytes={filters} filter_bytes_per_key=2.032\n"
OPEN = "OPEN run=1 cache=warm ns=9000000 heap_retained_bytes={held} heap_peak_bytes=1 rss_before_kib=na rss_after_kib=na page_cache_before_kib=na correctness_errors=0\nOPEN_SUMMARY cache=warm runs=1 min_ns=9000000 p50_ns=9000000 max_ns=9000000 correctness_errors=0\n"
APPEND = "CONFIG callers=20 rounds=50 interval_us=closed_loop offered_records_per_second=closed_loop roll_bytes=1048576 start_records=13000 start_segments=4\nOPEN ns=8000000\nAPPEND records=1000 records_per_second={rate} full=0 commit_errors=0 correctness_errors={errors} start_segments=4 end_segments=4\nMETRIC append_ns count=1000 p50=2600000 p99=5000000 p999=5100000 max=5200000\n"


def capture(errors=0):
    root = Path(tempfile.mkdtemp())
    keyed = FIXTURE.format(keys=1000, sealed=12000, filters=24384)
    for name, text in [("after-keyed", keyed), ("before-keyed", keyed), ("after-unkeyed", FIXTURE.format(keys=0, sealed=0, filters=0))]:
        (root / f"fixture-{name}.log").write_text(text)
    (root / "open").mkdir()
    (root / "open" / "keyed-warm.log").write_text(keyed + OPEN.format(held=544826))
    (root / "open" / "unkeyed-warm.log").write_text(FIXTURE.format(keys=0, sealed=0, filters=0) + OPEN.format(held=442622))
    for mode in ["closed", "paced"]:
        (root / "append" / mode).mkdir(parents=True)
        for side, rate in [("before", 6900), ("after", 6800)]:
            (root / "append" / mode / f"{side}-1.log").write_text(APPEND.format(rate=rate, errors=errors if side == "after" else 0))
    return root


class SummaryTest(unittest.TestCase):
    def test_reports_the_formula_beside_the_measured_heap_difference(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            m4_summary.main(capture())
        text = out.getvalue()
        self.assertIn("ADR 0048 rule 5, the filter part: 24384 bytes for 12000 sealed keys", text)
        self.assertIn("The difference, 102204 bytes, is the measured key-cache memory", text)
        self.assertIn("| keyed | cold | not run |", text)
        self.assertIn("after/before medians 0.986", text)

    def test_refuses_a_run_that_reported_an_error(self):
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(AssertionError):
            m4_summary.main(capture(errors=1))


if __name__ == "__main__":
    unittest.main()
