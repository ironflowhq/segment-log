#!/usr/bin/env python3
"""Check phase isolation and missing-sample semantics on known synthetic observations."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('summary', Path(__file__).with_name('summarize-resident-probe.py'))
summary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(summary)


class SummaryTests(unittest.TestCase):
    def test_held_and_contained_peaks_exclude_other_phases(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            fixtures = {
                'command.json': dict(mode='recover', records=8, keys=0, streams=1, group=2, payload=32, interval_ms=10),
                'status.json': {'failures': []},
                'events.json': [{'line': line} for line in ['begin\topen\t100', 'end\topen\t200\t100',
                                                            'begin\tread_one\t300', 'end\tread_one\t301\t1']],
                'samples.json': [
                    dict(start_ns=10, end_ns=20, rss_bytes=1000, kind='barrier', phase='baseline'),
                    dict(start_ns=50, end_ns=60, rss_bytes=9000, kind='active', phase='baseline'),
                    dict(start_ns=110, end_ns=120, rss_bytes=1500, kind='active', phase='open'),
                    dict(start_ns=190, end_ns=210, rss_bytes=8000, kind='active', phase='open'),
                    dict(start_ns=220, end_ns=230, rss_bytes=2000, kind='barrier', phase='recovered_held'),
                ],
            }
            for name, value in fixtures.items():
                (path / name).write_text(json.dumps(value))
            rows = {(r['phase'], r['kind']): r for r in summary.summarize(path)}
            self.assertEqual(rows['recovered_held', 'held']['delta_baseline_bytes'], 1000)
            peak = rows['open', 'sampled_peak_lower_bound']
            self.assertEqual((peak['rss_bytes'], peak['samples']), (1500, 1))
            self.assertEqual(peak['largest_start_gap_ns'], 90)
            missing = rows['read_one', 'sampled_peak_lower_bound']
            self.assertIsNone(missing['rss_bytes'])
            self.assertEqual(missing['samples'], 0)
            (path / 'status.json').unlink()
            with self.assertRaises(FileNotFoundError):
                summary.summarize(path)


if __name__ == '__main__':
    unittest.main()
