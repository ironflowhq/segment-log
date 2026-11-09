#!/usr/bin/env python3
"""Run the bounded resident-memory matrix serially during an authorized Mac interval."""
import argparse
import datetime
import json
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--sampler', type=Path, default=Path('scripts/mac-resident-probe.py'))
parser.add_argument('--binary', type=Path, default=Path('target/release/examples/resident_probe'))
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
base = dict(records=16384, keys=0, streams=1, group=256, payload=32)
cases = {
    'records': dict(base, records=4096, group=64),
    'keys': dict(base, keys=16384),
    'streams': dict(base, streams=1024),
    'groups': dict(base, group=16),
    'payload': dict(base, payload=256),
}


def environment(label):
    rows = []
    for command in [['uptime'], ['vm_stat'], ['sysctl', 'vm.swapusage'], ['memory_pressure', '-Q'], ['pmset', '-g', 'therm']]:
        r = subprocess.run(command, capture_output=True, text=True, timeout=15)
        rows.append(dict(command=command, returncode=r.returncode, stdout=r.stdout, stderr=r.stderr))
    (args.output / f'environment-{label}.json').write_text(json.dumps(
        dict(utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), commands=rows), indent=2))
    pressure = re.search(r'free percentage: (\d+)%', rows[3]['stdout'])
    if not pressure or int(pressure[1]) < 10:
        raise RuntimeError('pressure query unavailable or free percentage below 10%; stop matrix')


def trial(name, mode, dimensions, data, interval=10):
    output = args.output / name
    command = ['python3', str(args.sampler), str(args.binary), mode,
               '--data', str(data), '--output', str(output), '--interval-ms', str(interval)]
    for key, value in dimensions.items():
        command.extend([f'--{key}', str(value)])
    r = subprocess.run(command, capture_output=True, text=True, timeout=150)
    print(name, r.returncode, flush=True)
    with (args.output / 'runner.log').open('a') as log:
        log.write(name + '\n' + r.stdout + r.stderr)
    if r.returncode:
        raise RuntimeError(f'trial failed; preserve {output} and stop matrix')


environment('start')
for rep in range(1, 6):
    modes = ['empty', 'touch'] if rep % 2 else ['touch', 'empty']
    for mode in modes:
        dimensions = dict(base, records=8192, payload=1024) if mode == 'touch' else base
        trial(f'control-{rep}-{mode}', mode, dimensions, args.output / f'data-control-{rep}-{mode}')
        if mode == 'touch':
            import statistics
            samples = json.loads((args.output / f'control-{rep}-{mode}' / 'samples.json').read_text())
            median = lambda label: statistics.median(s['rss_bytes'] for s in samples
                                                    if s['kind'] == 'barrier' and s['phase'] == label)
            delta = median('touched_held') - median('baseline')
            if not 6 * 1024**2 <= delta <= 12 * 1024**2:
                raise RuntimeError(f'touch control out of 6–12 MiB bound: {delta}')
environment('controls')
for case, candidate in cases.items():
    for rep in range(1, 6):
        pair = [('base', base), ('candidate', candidate)]
        if rep % 2 == 0:
            pair.reverse()
        for side, dimensions in pair:
            name = f'{case}-{rep}-{side}'
            data = args.output / f'data-{name}'
            for mode in ['caller', 'append', 'recover']:
                trial(f'{name}-{mode}', mode, dimensions, data)
    environment(case)
for rep in range(1, 6):
    for interval in ([0, 10] if rep % 2 else [10, 0]):
        name = f'observer-{rep}-{interval}'
        for mode in ['append', 'recover']:
            trial(f'{name}-{mode}', mode, base, args.output / f'data-{name}', interval)
environment('end')
