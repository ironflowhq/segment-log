#!/usr/bin/env python3
"""Bounded Linux retained-history diagnostics; no implementation comparison."""
import argparse
import csv
import json
import os
from pathlib import Path
import platform
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary', type=Path, required=True)
p.add_argument('--output', type=Path, required=True, help='new output directory')
p.add_argument('--repetitions', type=int, default=3)
p.add_argument('--drop-guest-caches', action='store_true', help='global guest cache eviction, requires exclusive Linux host and root')
a = p.parse_args()
assert platform.system() == 'Linux'
assert a.repetitions > 0
a.output.mkdir(parents=True, exist_ok=False)
binary = str(a.binary.resolve())
commands = []

def command(cmd, **kwargs):
    commands.append(cmd)
    return subprocess.run(cmd, check=True, text=True, capture_output=True, **kwargs).stdout

metadata = {'platform': platform.platform(), 'binary': binary,
            'repetitions': a.repetitions, 'drop_guest_caches': a.drop_guest_caches}
for name, cmd in {'uptime': ['uptime'], 'cpu': ['lscpu'], 'memory': ['free', '-b'],
                  'disk': ['df', '-T', str(a.output)], 'processes': ['ps', '-eo', 'pid,comm,pcpu,pmem,args'],
                  'rust': ['rustc', '-Vv'], 'binary_hash': ['sha256sum', binary]}.items():
    metadata[name] = command(cmd)
(a.output / 'environment.json').write_text(json.dumps(metadata, indent=2))

cases = [(f'records_{n}', n, 0, 1, 100) for n in (1000, 10000, 100000)]
cases += [('keys_half', 10000, 5000, 1, 100), ('keys_all', 10000, 10000, 1, 100),
          ('streams_1000', 10000, 0, 1000, 100), ('groups_1000', 10000, 0, 1, 10),
          ('groups_10000', 10000, 0, 1, 1)]
with (a.output / 'phases.csv').open('w', newline='') as out:
    writer = csv.writer(out)
    writer.writerow(['case', 'repetition', 'cache', 'records', 'keys', 'streams', 'group_records',
                     'payload_bytes', 'phase', 'thread_allocated_bytes', 'process_live_delta_bytes',
                     'process_peak_extra_bytes', 'process_live_bytes', 'elapsed_ns', 'rss_kib', 'hwm_kib'])
    for name, n, keys, streams, group in cases:
        for rep in range(1, a.repetitions + 1):
            data = a.output / 'data'
            modes = ['seed', 'guest_warm'] + (['guest_dropped'] if a.drop_guest_caches else [])
            for mode in modes:
                if mode == 'guest_dropped':
                    command(['sync'])
                    Path('/proc/sys/vm/drop_caches').write_text('3\n')
                    commands.append(['write', '/proc/sys/vm/drop_caches', '3'])
                env = dict(os.environ, GROWTH_KEEP='1')
                if mode != 'seed':
                    env['GROWTH_REOPEN_ONLY'] = '1'
                text = command([binary, str(data), str(n), str(keys), str(streams), str(group), '32'], env=env)
                (a.output / f'{name}-{rep}-{mode}.txt').write_text(text)
                lines = text.splitlines()
                assert lines[-1] == f'validated_records={n},failures=0,groups={(n + group - 1) // group}', text
                rows = list(csv.reader(lines[:-1]))
                assert len(rows) == (12 if mode == 'seed' else 8), text
                for row in rows[1:]:
                    assert len(row) == 8, row
                    writer.writerow([name, rep, mode, n, keys, streams, group, 32, *row])
                out.flush()
            import shutil
            shutil.rmtree(data)
(a.output / 'commands.json').write_text(json.dumps(commands, indent=2))
(a.output / 'uptime-end.txt').write_text(command(['uptime']))
print(f'validated_runs={len(cases) * a.repetitions * len(modes)},failures=0')
