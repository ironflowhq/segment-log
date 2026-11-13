import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "key_page_alignment", Path(__file__).with_name("key-page-alignment.py")
)
alignment = importlib.util.module_from_spec(spec)
spec.loader.exec_module(alignment)


class AlignmentTests(unittest.TestCase):
    def test_offsets_are_distinct_pages_aligned_or_inside_a_page(self):
        size = 1000 * alignment.PAGE
        aligned = alignment.offsets(size, 100, 7, True)
        unaligned = alignment.offsets(size, 100, 7, False)
        self.assertTrue(all(o % alignment.PAGE == 0 for o in aligned))
        self.assertTrue(all(o % alignment.PAGE != 0 for o in unaligned))
        self.assertEqual(len({o // alignment.PAGE for o in unaligned}), 100)
        # Every read of a full page stays inside the file.
        self.assertTrue(all(o + alignment.PAGE <= size for o in aligned + unaligned))

    def test_summary_takes_the_median_of_each_variant(self):
        rows = [
            "aligned-normal\t1\t10\t100\t200\t300\t150\t10\t40",
            "aligned-normal\t2\t10\t300\t400\t500\t350\t30\t120",
            "aligned-normal\t3\t10\t200\t300\t400\t250\t20\t80",
            "unaligned-normal\t1\t10\t900\t900\t900\t900\t40\t160",
        ]
        self.assertEqual(
            alignment.summarize(rows),
            [
                "variant\ttrials\tp50_ns\tp90_ns\tp99_ns\tmean_ns\tdevice_reads\tdevice_kib",
                "aligned-normal\t3\t200\t300\t400\t250\t20\t80",
                "unaligned-normal\t1\t900\t900\t900\t900\t40\t160",
            ],
        )


if __name__ == "__main__":
    unittest.main()
