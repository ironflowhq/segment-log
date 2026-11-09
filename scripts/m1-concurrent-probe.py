#!/usr/bin/env python3
"""Serial Mac diagnostic runner. Build separately and obtain an exclusive interval first."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import time


def capture(command):
    try:
        r = subprocess.run(command, text=True, capture_output=True, timeout=20)
        return {'command': command, 'status': r.returncode, 'stdout': r.stdout, 'stderr': r.stderr}
    except subprocess.TimeoutExpired as e:
        return {'command': command, 'status': 'timeout', 'stdout': str(e.stdout), 'stderr': str(e.stderr)}


def environment():
    return {name: capture(command) for name, command in {
        'date': ['date', '-u'], 'load': ['uptime'],
        'thermal': ['pmset', '-g', 'therm'],
        'swap': ['sysctl', 'vm.swapusage'], 'vm_stat': ['vm_stat'],
        'memory_pressure': ['memory_pressure', '-Q'],
        'processes': ['ps', '-axo', 'pid,ppid,%cpu,rss,comm'],
    }.items()}


def pressure_ok(observation):
    text = observation['memory_pressure']['stdout']
    match = re.search(r'System-wide memory free percentage: (\d+)%', text)
    return observation['memory_pressure']['status'] == 0 and match is not None and int(match[1]) >= 30


def cases(reps):
    for keyed in ('unkeyed', 'keyed'):
        for rep in range(1, reps + 1):
            modes = ('append', 'concurrent') if rep % 2 else ('concurrent', 'append')
            if keyed == 'unkeyed' and rep % 2 == 0:
                yield keyed, rep, 'concurrent', 'timing', False
            for mode in modes:
                timings = ('timing', 'untimed') if rep % 2 else ('untimed', 'timing')
                if keyed == 'keyed':
                    timings = ('timing',)
                for timing in timings:
                    yield keyed, rep, mode, timing, True
            if keyed == 'unkeyed' and rep % 2:
                yield keyed, rep, 'concurrent', 'timing', False


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--repetitions', type=int, default=5)
    p.add_argument('--seed', type=int, default=1024)
    p.add_argument('--added', type=int, default=32768)
    p.add_argument('--timeout', type=int, default=120)
    p.add_argument('--plan-only', action='store_true')
    a = p.parse_args()
    assert a.repetitions > 0 and a.seed > 0 and a.seed % 32 == 0
    assert a.added > 0 and a.added % 128 == 0 and a.timeout > 0
    plan = list(cases(a.repetitions))
    if a.plan_only:
        print(json.dumps(plan, indent=2))
        return
    assert platform.system() == 'Darwin', 'Mac-only probe'
    a.output.mkdir(parents=True, exist_ok=False)
    binary = a.binary.resolve()
    metadata = {'platform': platform.platform(), 'binary': str(binary),
                'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                'arguments': {k: str(v) for k, v in vars(a).items()},
                'cache': 'No cache eviction; fresh directories, naturally warm system cache.',
                'plan': plan, 'environment': environment()}
    for name, command in {
        'source': ['git', 'rev-parse', 'HEAD'], 'status': ['git', 'status', '--short'],
        'diff': ['git', 'diff'], 'rust': ['rustc', '-Vv'],
        'cpu': ['sysctl', 'machdep.cpu.brand_string', 'hw.ncpu', 'hw.memsize'],
        'os': ['sw_vers'], 'filesystem': ['df', '-h', str(a.output)],
    }.items():
        metadata[name] = capture(command)
    metadata['probe_sources'] = {}
    for path in ('segment-log-core/examples/concurrent_probe.rs', 'scripts/m1-concurrent-probe.py'):
        source = Path(path).read_bytes()
        (a.output / Path(path).name).write_bytes(source)
        metadata['probe_sources'][path] = hashlib.sha256(source).hexdigest()
    (a.output / 'environment.json').write_text(json.dumps(metadata, indent=2))
    results = []
    for keyed, rep, mode, timing, sample in plan:
        name = f'{keyed}-{rep}-{mode}-{timing}-rss{int(sample)}'
        dest = a.output / name
        dest.mkdir()
        before = environment()
        (dest / 'before.json').write_text(json.dumps(before, indent=2))
        if not pressure_ok(before):
            raise SystemExit('Pressure observation missing or free-percentage label below 30%; stopped before run.')
        command = ['/usr/bin/time', '-l', str(binary), str(dest / 'data'), mode, keyed,
                   'default', str(a.seed), str(a.added), timing]
        (dest / 'command.json').write_text(json.dumps(command))
        started = time.monotonic()
        samples = []
        timed_out = False
        with (dest / 'stdout.txt').open('w') as out, (dest / 'stderr.txt').open('w') as err:
            proc = subprocess.Popen(command, stdout=out, stderr=err, start_new_session=True)
            # time(1) is the parent; find only this invocation's direct child.
            child = None
            while proc.poll() is None:
                if time.monotonic() - started > a.timeout:
                    timed_out = True
                    os.killpg(proc.pid, signal.SIGKILL)
                    break
                if child is None:
                    r = capture(['pgrep', '-P', str(proc.pid)])
                    ids = r['stdout'].split()
                    if ids:
                        assert len(ids) == 1, ids
                        child = int(ids[0])
                if sample and child is not None:
                    t = time.monotonic()
                    result = capture(['ps', '-p', str(child), '-o', 'pid=,rss=,%cpu=,time='])
                    progress = (dest / 'stdout.txt').read_text().splitlines()
                    marks = [line for line in progress if line.startswith(('phase,', 'stage,'))]
                    samples.append({'seconds': t - started, 'cost_seconds': time.monotonic() - t,
                                    'last_marker': marks[-1] if marks else '', **result})
                time.sleep(0.25)
            status = proc.wait()
        (dest / 'rss.json').write_text(json.dumps(samples, indent=2))
        after = environment()
        (dest / 'after.json').write_text(json.dumps(after, indent=2))
        text = (dest / 'stdout.txt').read_text()
        expected = a.seed + a.added
        validation = [line for line in text.splitlines() if line.startswith('validation,')]
        valid = status == 0 and len(validation) == 1 and validation[0].split(',')[1] == str(expected) and validation[0].endswith(',0')
        read_rows = [line.split(',') for line in text.splitlines() if line.startswith('reads,')]
        if mode == 'concurrent':
            valid &= len(read_rows) == 4 and all(int(r[2]) > 0 and r[3:5] == ['0', '0'] for r in read_rows)
        result = {'case': name, 'status': status, 'timeout': timed_out, 'validated': valid,
                  'wall_seconds': time.monotonic() - started, 'child_pid': child}
        results.append(result)
        (a.output / 'results.json').write_text(json.dumps(results, indent=2))
        print(json.dumps(result), flush=True)
        if not pressure_ok(after):
            raise SystemExit('Pressure observation missing or free-percentage label below 30%; stopped after run.')
        if not valid:
            raise SystemExit('Run failed; raw output and dataset retained. Investigate before continuing.')


if __name__ == '__main__':
    main()
