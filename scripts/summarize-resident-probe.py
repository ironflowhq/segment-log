#!/usr/bin/env python3
"""Reduce retained RSS trial directories without treating missing peak samples as zero."""
import argparse
import csv
import json
from pathlib import Path
import statistics


def summarize(path):
    command = json.loads((path / 'command.json').read_text())
    status = json.loads((path / 'status.json').read_text())
    samples = json.loads((path / 'samples.json').read_text())
    events = json.loads((path / 'events.json').read_text())
    labels = sorted({s['phase'] for s in samples if s.get('kind') == 'barrier'})
    base = [s['rss_bytes'] for s in samples if s.get('kind') == 'barrier'
            and s['phase'] == 'baseline' and 'rss_bytes' in s]
    prefix = {'run': path.name, 'mode': command['mode'],
              **{k: command[k] for k in ['records', 'keys', 'streams', 'group', 'payload', 'interval_ms']},
              'failures': len(status['failures'])}
    rows = []
    for label in labels:
        readings = [s['rss_bytes'] for s in samples if s.get('kind') == 'barrier'
                    and s['phase'] == label and 'rss_bytes' in s]
        rows.append({**prefix, 'phase': label, 'kind': 'held', 'samples': len(readings),
                     'rss_bytes': statistics.median(readings) if readings else None,
                     'delta_baseline_bytes': statistics.median(readings) - statistics.median(base)
                     if readings and base else None})
    starts = {}
    for event in events:
        parts = event['line'].split('\t')
        if parts[0] == 'begin':
            starts[parts[1]] = int(parts[2])
        if parts[0] == 'end':
            label = parts[1]
            start, end = starts[label], int(parts[2])
            if end < start:
                raise ValueError('wall clock moved backwards')
            contained = [s for s in samples if 'rss_bytes' in s
                         and start <= s['start_ns'] <= s['end_ns'] <= end]
            stamps = [start, *[s['start_ns'] for s in contained], end]
            rows.append({**prefix, 'phase': label, 'kind': 'sampled_peak_lower_bound',
                         'samples': len(contained),
                         'rss_bytes': max((s['rss_bytes'] for s in contained), default=None),
                         'elapsed_ns': int(parts[3]),
                         'largest_start_gap_ns': max(b - a for a, b in zip(stamps, stamps[1:]))})
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    trials = sorted(p.parent for p in args.directory.rglob('status.json'))
    if not trials:
        parser.error('no trial status files')
    rows = [row for path in trials for row in summarize(path)]
    fields = ['run', 'mode', 'records', 'keys', 'streams', 'group', 'payload', 'interval_ms',
              'failures', 'phase', 'kind', 'samples', 'rss_bytes', 'delta_baseline_bytes',
              'elapsed_ns', 'largest_start_gap_ns']
    with args.output.open('w') as output:
        writer = csv.DictWriter(output, fields)
        writer.writeheader()
        writer.writerows(rows)


if __name__ == '__main__':
    main()
