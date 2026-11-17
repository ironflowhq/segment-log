import contextlib
import importlib.util
import io
import re
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("m7_capacity", Path(__file__).with_name("m7-capacity.py"))
cap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cap)

FORMAT = Path(__file__).resolve().parents[1] / "docs" / "architecture" / "format.md"


def profile(**kw):
    p = {"payload": 10, "stream": 1, "key": 1, "extra": 0, "roll_bytes": 100, "streams": 0, "per_group": 2}
    p.update(kw)
    return p


def run(*argv):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = cap.main(list(argv))
    return code, dict(line.split("=", 1) for line in out.getvalue().splitlines())


class SizingTests(unittest.TestCase):
    def test_widths_follow_the_format_version_in_format_md(self):
        # A new format_version must revisit every width here, including GROUP_EXTRA.
        version = re.search(r"Status: `format_version` (\d+)", FORMAT.read_text()).group(1)
        self.assertEqual(cap.FORMAT_VERSION, int(version))

    def test_group_frame_of_the_provisional_profile(self):
        self.assertEqual(cap.entry_bytes(256, 16, 16), 304)
        self.assertEqual(cap.records_per_group(16384, 256, 16, 16), 53)
        self.assertEqual(cap.group_frame(53, 256, 16, 16), 8 + 24 + 8 + 53 * 304)
        self.assertEqual(cap.group_frame(53, 256, 16, 16, extra=0), 8 + 24 + 53 * 304)

    def test_closing_run_without_keys(self):
        # key filter and key index are body headers alone; one footer entry; the 40-byte locator
        self.assertEqual(cap.closing_run(0, 1, 16), 32 + 32 + (32 + 8 + 28 + 16) + 40)

    def test_closing_run_with_keys_takes_the_worst_padding(self):
        filt = 32 + 8 * 2 + 64 * 8 + 4095
        index = 32 + 4096 * 2
        footer = 32 + 8 + 256 * (28 + 16)
        self.assertEqual(cap.closing_run(256, 256, 16), filt + index + footer + 40)

    def test_encoded_size_counts_each_segment_closed(self):
        # roll 100: one 88-byte group per segment; 3 records make a full segment and a short one
        closing2 = (32 + 8 + 64 + 4095) + (32 + 4096) + (40 + 2 * 29) + 40
        closing1 = (32 + 8 + 64 + 4095) + (32 + 4096) + (40 + 29) + 40
        want = (64 + 88 + closing2) + (64 + 60 + closing1)
        self.assertEqual(cap.encoded_bytes(3, profile()), want)
        self.assertEqual(cap.encoded_bytes(0, profile()), 0)

    def test_capped_streams_shrink_the_footer(self):
        self.assertEqual(cap.encoded_bytes(2, profile()) - cap.encoded_bytes(2, profile(streams=1)), 29)

    def test_unkeyed_records_have_empty_key_frames(self):
        p = profile(key=0)
        self.assertEqual(cap.encoded_bytes(1, p), 64 + (32 + 27) + 32 + 32 + (40 + 29) + 40)

    def test_the_keyed_limit_rolls_before_the_size_does(self):
        self.assertEqual(cap.groups_per_segment(4000 << 20, 16144, 53, 16, 16, 1), 1_044_225 // 53)
        self.assertEqual(cap.groups_per_segment(4000 << 20, 16144, 53, 0, 16, 1), -(-((4000 << 20) - 64) // 16144))

    def test_the_footer_limit_rolls_before_a_large_roll_size(self):
        # one stream per record: (16 MiB - 32) // 44 = 381,300 streams, 7,194 groups of 53
        self.assertEqual(cap.groups_per_segment(1 << 30, 16144, 53, 16, 16, 0), 381_300 // 53)
        self.assertEqual(cap.groups_per_segment(1 << 30, 16144, 53, 0, 16, 1000), -(-((1 << 30) - 64) // 16144))
        self.assertEqual(cap.groups_per_segment(64 << 20, 16144, 53, 16, 16, 0), -(-((64 << 20) - 64) // 16144))

    def test_records_for_is_the_largest_that_fits(self):
        p = profile(roll_bytes=64 << 20, per_group=53, payload=256, stream=16, key=16)
        n = cap.records_for(1 << 30, p)
        self.assertLessEqual(cap.encoded_bytes(n, p), 1 << 30)
        self.assertGreater(cap.encoded_bytes(n + 1, p), 1 << 30)


class CliTests(unittest.TestCase):
    def test_thresholds_and_free_space(self):
        code, out = run("--records", "0", "--logs", "8", "--rate", "1000", "--response", "60", "--avail-ratio", "0.5")
        self.assertEqual(code, 0)
        self.assertEqual(out["guest_alert_threshold_bytes"], "60000")
        self.assertEqual(out["host_alert_threshold_bytes"], "480000")
        self.assertEqual(out["guest_free_needed_bytes"], "60000")
        self.assertEqual(out["image_bytes"], str(1 << 30))  # rounded up to a whole GiB
        self.assertEqual(out["host_free_needed_bytes"], str(8 * (1 << 30) + 480000))

    def test_allocated_size_applies_ratio_then_headroom(self):
        _, out = run("--records", "53", "--ratio", "1.5", "--headroom", "0.2")
        encoded = int(out["encoded_bytes_per_log"])
        self.assertEqual(int(out["planned_allocated_bytes_per_log"]), -(-encoded * 18 // 10))

    def test_a_host_that_is_too_small_fails(self):
        code, out = run("--history-bytes", "50GiB", "--host-free", "100GiB")
        self.assertEqual((code, out["host_fits"]), (1, "no"))
        code, out = run("--history-bytes", "50GiB", "--host-free", "894GiB")
        self.assertEqual((code, out["host_fits"]), (0, "yes"))

    def test_byte_units(self):
        self.assertEqual(cap.bytes_arg("50GiB"), 50 << 30)
        self.assertEqual(cap.bytes_arg("960GB"), 960 * 10**9)
        self.assertEqual(cap.bytes_arg("4096"), 4096)
        with self.assertRaises(Exception):
            cap.bytes_arg("5XB")


if __name__ == "__main__":
    unittest.main()
