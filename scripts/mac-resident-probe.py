#!/usr/bin/env python3
"""Run one phase-isolated Mac probe; retain events, RSS samples, errors, and optional VM maps."""
import argparse
import json
import os
from pathlib import Path
import resource
import selectors
import subprocess
import time


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    command = [str(args.binary.resolve()), args.mode, str(args.data.resolve()),
               str(args.records), str(args.keys), str(args.streams), str(args.group), str(args.payload)]
    (args.output / 'command.json').write_text(json.dumps(vars(args), default=str, indent=2))
    samples, events, failures = [], [], []
    start_usage = resource.getrusage(resource.RUSAGE_SELF)
    start_children = resource.getrusage(resource.RUSAGE_CHILDREN)
    with (args.output / 'stderr.txt').open('w') as stderr, (args.output / 'stdout.txt').open('w') as stdout:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr)
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        pending = b''
        barrier = None
        phase = 'startup'
        deadline = time.monotonic() + args.timeout
        next_sample = time.monotonic()
        result = None
        barrier_samples = 0
        ps_ns = 0
        ps_user_s = 0.0
        ps_system_s = 0.0

        def sample(label):
            nonlocal ps_ns, ps_user_s, ps_system_s
            usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
            start = time.time_ns()
            observed = subprocess.run(['ps', '-o', 'rss=', '-p', str(process.pid)],
                                      capture_output=True, text=True, timeout=5)
            end = time.time_ns()
            usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
            ps_user_s += usage_after.ru_utime - usage_before.ru_utime
            ps_system_s += usage_after.ru_stime - usage_before.ru_stime
            ps_ns += end - start
            row = {'start_ns': start, 'end_ns': end, 'phase': label,
                   'kind': 'barrier' if barrier is not None else 'active',
                   'returncode': observed.returncode, 'stdout': observed.stdout,
                   'stderr': observed.stderr}
            if observed.returncode == 0 and observed.stdout.strip().isdigit():
                row['rss_bytes'] = int(observed.stdout) * 1024
            else:
                failures.append({'kind': 'sample', **row})
            samples.append(row)

        try:
            while process.poll() is None or selector.get_map():
                now = time.monotonic()
                if now > deadline:
                    raise TimeoutError('probe deadline exceeded')
                for key, _ in selector.select(timeout=0.005):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    pending += chunk
                    while b'\n' in pending:
                        line, pending = pending.split(b'\n', 1)
                        line = line.decode()
                        stdout.write(line + '\n')
                        stdout.flush()
                        parts = line.split('\t')
                        events.append({'received_ns': time.time_ns(), 'line': line})
                        if parts[0] == 'barrier':
                            phase = parts[1]
                            barrier = time.monotonic() + args.settle_ms / 1000
                            barrier_samples = 0
                            next_sample = barrier
                        elif parts[0] == 'begin':
                            phase = parts[1]
                        elif parts[0] == 'result':
                            result = {k: int(v) for k, v in (p.split('=') for p in parts[1:])}
                now = time.monotonic()
                if barrier is not None and now >= next_sample:
                    sample(phase)
                    barrier_samples += 1
                    next_sample = time.monotonic() + 0.02
                    if barrier_samples == 3:
                        if args.maps and phase in {'baseline', 'empty_held', 'caller_held', 'recovered_held',
                                                   'appended_held', 'validation_held', 'engine_dropped',
                                                   'touched_held', 'touched_dropped'}:
                            for tool, cmd in [('vmmap', ['vmmap', '-summary']), ('footprint', ['footprint', '-p'])]:
                                r = subprocess.run([*cmd, str(process.pid)], capture_output=True, text=True, timeout=15)
                                (args.output / f'{phase}-{tool}.txt').write_text(
                                    f'returncode={r.returncode}\n' + r.stdout + r.stderr)
                                if r.returncode:
                                    failures.append({'kind': tool, 'phase': phase, 'returncode': r.returncode})
                        process.stdin.write(b'go\n')
                        process.stdin.flush()
                        barrier = None
                        next_sample = time.monotonic() + args.interval_ms / 1000
                elif barrier is None and args.interval_ms > 0 and now >= next_sample and process.poll() is None:
                    sample(phase)
                    next_sample = time.monotonic() + args.interval_ms / 1000
            code = process.wait(timeout=5)
            expected = args.records if args.mode in {'append', 'recover'} else 0
            if (code or result is None or result.get('failures') != 0
                    or result.get('admission_errors') != 0 or result.get('commit_errors') != 0
                    or result.get('validated') != expected):
                failures.append({'kind': 'validation', 'returncode': code, 'result': result, 'expected': expected})
            if args.mode == 'append' and (result is None or result.get('committed') != args.records):
                failures.append({'kind': 'commit_count', 'result': result})
        except Exception as error:
            failures.append({'kind': 'runner', 'error': repr(error)})
            if process.poll() is None:
                process.kill()
            code = process.wait(timeout=5)
        finally:
            selector.close()
            process.stdin.close()
            process.stdout.close()
            (args.output / 'samples.json').write_text(json.dumps(samples, indent=2))
            (args.output / 'events.json').write_text(json.dumps(events, indent=2))
    end_usage = resource.getrusage(resource.RUSAGE_SELF)
    end_children = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = lambda a, b: b.ru_utime + b.ru_stime - a.ru_utime - a.ru_stime
    summary = {'pid': process.pid, 'returncode': code, 'result': result, 'failures': failures,
               'samples': len(samples), 'ps_elapsed_ns': ps_ns,
               'observer_cpu_s': cpu(start_usage, end_usage),
               'observer_user_s': end_usage.ru_utime - start_usage.ru_utime,
               'observer_system_s': end_usage.ru_stime - start_usage.ru_stime,
               'ps_user_s': ps_user_s, 'ps_system_s': ps_system_s,
               'workload_cpu_s': None if args.maps else cpu(start_children, end_children) - ps_user_s - ps_system_s,
               'child_and_tools_cpu_s': cpu(start_children, end_children)}
    (args.output / 'status.json').write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary))
    return bool(failures)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('mode', choices=['empty', 'caller', 'append', 'recover', 'touch'])
    parser.add_argument('--data', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--records', type=int, default=4096)
    parser.add_argument('--keys', type=int, default=0)
    parser.add_argument('--streams', type=int, default=1)
    parser.add_argument('--group', type=int, default=256)
    parser.add_argument('--payload', type=int, default=32)
    parser.add_argument('--interval-ms', type=float, default=10, help='delay after each active ps call; 0 disables active sampling')
    parser.add_argument('--settle-ms', type=float, default=100)
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--maps', action='store_true', help='diagnostic only, outside comparison runs')
    args = parser.parse_args()
    if min(args.records, args.streams, args.group, args.payload) <= 0 or args.interval_ms < 0 or args.settle_ms < 0:
        parser.error('dimensions must be positive and sampling delays nonnegative')
    raise SystemExit(run(args))


if __name__ == '__main__':
    main()
