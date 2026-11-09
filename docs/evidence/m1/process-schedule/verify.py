import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile


def run_case(name, source, patch, repetitions, seed, expect_failure, output):
    root = Path(__file__).resolve().parents[4]
    channel = re.search(r'channel = "([^"]+)"', (root / 'rust-toolchain.toml').read_text())[1]
    with tempfile.TemporaryDirectory(prefix='segment-log-process-evidence-') as scratch:
        scratch = Path(scratch)
        test = scratch / 'process_crash.rs'
        test.write_text(source.read_text())
        subprocess.run(['patch', '--batch', str(test), str(patch)], check=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        dependency = str(root / 'segment-log-core').replace('\\', '\\\\').replace('"', '\\"')
        (scratch / 'Cargo.toml').write_text(
            '[package]\nname = "segment-log-process-evidence"\nversion = "0.0.0"\nedition = "2024"\n'
            '[dependencies]\nsegment-log-core = { path = "' + dependency + '" }\n'
            '[[test]]\nname = "process_crash"\npath = "process_crash.rs"\n'
        )
        command = ['cargo', '+' + channel, 'test', '--manifest-path', str(scratch / 'Cargo.toml'),
                   '--test', 'process_crash', 'killed_writers_preserve_the_recorded_contract',
                   '--', '--exact', '--nocapture']
        environment = dict(os.environ, SEGMENT_LOG_PROCESS_SEED=str(seed),
                           SEGMENT_LOG_PROCESS_REPETITIONS=str(repetitions))
        log = output / (name + '.log')
        with log.open('w') as stream:
            stream.write(f'seed={seed} repetitions={repetitions}\n{command!r}\n')
            stream.flush()
            result = subprocess.run(command, env=environment, cwd=scratch, stdout=stream,
                                    stderr=subprocess.STDOUT, timeout=max(60, repetitions * 30))
            stream.write(f'Exit code: {result.returncode}\n')
        text = log.read_text()
        if expect_failure:
            assert result.returncode == 101 and 'abandoned reply must exercise unknown outcomes' in text, text
        else:
            assert result.returncode == 0, text
        print(f'{name}: expected result observed; {log}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--controlled-repetitions', type=int, default=1)
    parser.add_argument('--output', type=Path, default=Path('.audit/process-evidence'))
    args = parser.parse_args()
    assert 1 <= args.controlled_repetitions <= 10000
    args.output.mkdir(parents=True, exist_ok=True)
    here = Path(__file__).resolve().parent
    run_case('before-gate', here / 'reviewed-pre-B0-process_crash.rs',
             here / 'schedule-only.patch', 1, 808, True, args.output.resolve())
    run_case('after-gate', here.parents[3] / 'segment-log-core/tests/process_crash.rs',
             here / 'fixed-controlled-schedule.patch', args.controlled_repetitions,
             30000, False, args.output.resolve())
