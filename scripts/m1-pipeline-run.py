#!/usr/bin/env python3
"""Run one bounded Mac pipeline observation. Build and schedule runs separately."""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=pathlib.Path)
parser.add_argument("output", type=pathlib.Path)
parser.add_argument("--callers", type=int, default=8)
parser.add_argument("--rounds", type=int, default=8)
parser.add_argument("--trace", action="store_true")
args = parser.parse_args()
assert sys.platform == "darwin", "Mac diagnostic only"
assert args.callers > 0 and args.rounds > 0
args.output.mkdir(parents=True, exist_ok=False)
output = args.output.resolve()
binary = args.binary.resolve()


def host(label):
    with (output / f"host-{label}.txt").open("w") as out:
        for command in [["date", "-u"], ["uptime"], ["ps", "-Ao", "pid,ppid,%cpu,rss,comm", "-r"],
                        ["top", "-l", "2", "-s", "1", "-n", "0"],
                        ["pmset", "-g", "therm"], ["vm_stat"], ["df", "-h", str(output)]]:
            out.write(f"$ {command!r}\n")
            out.flush()
            subprocess.run(command, stdout=out, stderr=subprocess.STDOUT, check=False)


env = os.environ.copy()
for key in list(env):
    if key.startswith("SEGMENT_LOG_"):
        del env[key]
env.update(SEGMENT_LOG_BENCH_CALLERS=str(args.callers), SEGMENT_LOG_BENCH_ROUNDS=str(args.rounds),
           SEGMENT_LOG_DEVICE_ROUNDS="16", SEGMENT_LOG_BENCH_STORAGE="real",
           SEGMENT_LOG_EVIDENCE_DIR=str(output / "data"))
if args.trace:
    env["SEGMENT_LOG_PIPELINE_OUT"] = str(output / "requests.tsv")
command = ["/usr/bin/time", "-l", str(binary), "--exact",
           "acceptance::evidence::measured_appends_survive_reopen", "--nocapture", "--test-threads=1"]
manifest = dict(command=command, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                env={key: value for key, value in env.items() if key.startswith("SEGMENT_LOG_")},
                cache="fresh directory; no cache eviction; warm toolchain and OS cache; same-process reopen",
                wall_start=time.time())
host("before")
with (output / "run.log").open("w") as out:
    process = subprocess.Popen(command, env=env, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
    manifest["pid"] = process.pid
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    try:
        manifest["exit"] = process.wait(timeout=240)
    except subprocess.TimeoutExpired:
        import signal
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        manifest["exit"] = "timeout"
manifest["wall_end"] = time.time()
host("after")
(output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
if manifest["exit"] != 0:
    raise SystemExit(f"run failed: {manifest['exit']}; retain {output}")
log = (output / "run.log").read_text()
assert "test result: ok. 1 passed" in log
assert f"APPEND records={args.callers * args.rounds} " in log
assert "other_admission_errors=0 commit_errors=0 correctness_errors=0" in log
assert "REOPEN " in log and "correctness_errors=0" in log
if args.trace:
    with (output / "summary.json").open("w") as out:
        subprocess.run([sys.executable, str(pathlib.Path(__file__).with_name("m1-pipeline-summary.py")),
                        str(output / "requests.tsv")], stdout=out, check=True)
print(output)
