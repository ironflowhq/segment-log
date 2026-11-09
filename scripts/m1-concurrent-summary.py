#!/usr/bin/env python3
"""Validate retained probe counts and write run/stage/bucket summaries. No pooling across runs."""
import csv
import json
import math
from pathlib import Path
import sys


def quantile(bins, fraction):
    target = math.ceil(sum(n for low, high, n in bins) * fraction)
    seen = 0
    for low, high, n in sorted(bins):
        seen += n
        if seen >= target:
            return low, high
    return None, None


def summarize(root):
    results = json.loads((root / 'results.json').read_text())
    metadata = json.loads((root / 'environment.json').read_text())
    seed = int(metadata['arguments']['seed'])
    added = int(metadata['arguments']['added'])
    run_rows, stage_rows, distribution_rows = [], [], []
    for result in results:
        assert result['validated'] and result['status'] == 0 and not result['timeout'], result
        case = result['case']
        keyed, rep, mode, timing, sample = case.split('-')
        raw = list(csv.reader((root / case / 'stdout.txt').read_text().splitlines()))
        assert not any(r[0] == 'failure' for r in raw), case
        measurements = [r for r in raw if r[0] == 'measurement']
        assert len(measurements) == 1
        elapsed, committed, prefix, watermark = map(int, measurements[0][1:])
        assert committed == added and watermark == seed + added and 0 <= prefix <= watermark
        validation = [r for r in raw if r[0] == 'validation']
        assert len(validation) == 1 and int(validation[0][1]) == watermark and validation[0][3] == '0'
        assert int(validation[0][2]) == (watermark if mode == 'concurrent' else 0)
        assert any(r[0] == 'success_counts' for r in raw)
        stages = {int(r[1]): list(map(int, r[2:])) for r in raw if r[0] == 'stage'}
        reads = {int(r[1]): list(map(int, r[2:])) for r in raw if r[0] == 'reads'}
        assert set(stages) == set(range(4))
        totals = {}
        for r in (r for r in raw if r[0] == 'distribution'):
            stage, kind, count, total, maximum = int(r[1]), r[2], *map(int, r[3:])
            bins = [tuple(map(int, b[3:])) for b in raw if b[0] == 'bin' and b[1:3] == r[1:3]]
            assert sum(n for low, high, n in bins) == count
            assert all(low <= high and n > 0 for low, high, n in bins)
            assert sum(low*n for low, high, n in bins) <= total <= sum(high*n for low, high, n in bins)
            if count:
                assert min(low for low, high, n in bins) <= maximum <= max(high for low, high, n in bins)
            totals.setdefault(kind, []).extend(bins)
            distribution_rows.append([case, stage, kind, count, total, maximum,
                                      *quantile(bins, .5), *quantile(bins, .99)])
        for stage, (ns, n, w) in stages.items():
            assert n == added // 4 * (stage + 1) and w == seed + n
            count, errors, mismatches, records, gap = reads.get(stage, [0]*5)
            assert errors == 0 and mismatches == 0
            if mode == 'concurrent':
                assert count > 0, f'{case}: no reader overlap at cycle-start stage {stage}'
            if timing == 'timing':
                hist_count = sum(int(r[3]) for r in raw if r[0] == 'distribution' and int(r[1]) == stage and r[2] != 'append')
                assert hist_count == count
            stage_rows.append([case, stage, ns, seed + added // 4 * stage, w, count, records, gap])
        append_count = sum(n for low, high, n in totals['append'])
        assert append_count == (added // 32 if timing == 'timing' else 0)
        counts = sum(v[0] for v in reads.values())
        rss = json.loads((root / case / 'rss.json').read_text())
        active = [s for s in rss if s['status'] == 0 and s['last_marker'] and s['last_marker'] != 'phase,measurement_end']
        rss_kib = [int(s['stdout'].split()[1]) for s in active]
        run_rows.append([case, elapsed, added * 1e9 / elapsed, seed // 32, added // 32, counts,
                         prefix, watermark, counts * 1e9 / elapsed,
                         *quantile(totals['append'], .5), *quantile(totals['append'], .99),
                         min(rss_kib, default=''), max(rss_kib, default=''), len(active),
                         sum(s['cost_seconds'] for s in rss), 0, 0, 0])
    for name, header, rows in (
        ('runs.csv', ['case','append_wall_ns','records_per_second','seed_commits','measured_commits','reader_calls',
                      'reader_prefix','final_watermark','reader_calls_per_append_second','append_p50_low_ns','append_p50_high_ns',
                      'append_p99_low_ns','append_p99_high_ns','rss_min_kib','rss_max_kib','active_rss_samples',
                      'sampler_wall_seconds','append_refusals','commit_errors','read_errors'], run_rows),
        ('stages.csv', ['case','cycle_start_stage','append_wall_ns','history_start','history_end','reader_calls','reader_records','max_cycle_gap_ns'], stage_rows),
        ('distributions.csv', ['case','cycle_start_stage','kind','count','sum_ns','max_ns','p50_low_ns','p50_high_ns','p99_low_ns','p99_high_ns'], distribution_rows),
    ):
        with (root / name).open('w') as f:
            writer = csv.writer(f, lineterminator="\n")
            writer.writerow(header)
            writer.writerows(rows)
    print(f'validated_runs={len(results)},measurement_commits={sum(r[4] for r in run_rows)},reader_calls={sum(r[5] for r in run_rows)},failures=0')


if __name__ == '__main__':
    summarize(Path(sys.argv[1]))
