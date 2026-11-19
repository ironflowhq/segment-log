"""Resident memory per stream (issue 156), from resident_probe's barriers.

    streams.py RESIDENT_PROBE OUT_DIR REPS

Runs resident_probe on 1,000,000 records with 32-byte payloads, no keys, in groups of 1,000, over
1, 1,000, 100,000 and 1,000,000 streams: an append process, then a recover process on the log it
wrote. At each barrier it reads the process's resident set with ps, then lets it go on. Writes
OUT_DIR/streams.tsv.
"""
import os
import shutil
import subprocess
import sys

probe, out, reps = sys.argv[1], sys.argv[2], int(sys.argv[3])
RECORDS, GROUP, PAYLOAD = 1_000_000, 1_000, 32
STREAMS = [1, 1_000, 100_000, 1_000_000]


def resident_kib(pid):
    return int(subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True, check=True).stdout)


def run(mode, log):
    args = [probe, mode, log, str(RECORDS), "0", str(streams), str(GROUP), str(PAYLOAD)]
    p = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    samples = {}
    for line in p.stdout:
        fields = line.rstrip("\n").split("\t")
        if fields[0] == "barrier":
            samples[fields[1]] = resident_kib(p.pid)
            p.stdin.write("go\n")
            p.stdin.flush()
        elif fields[0] == "result" and "failures=0" not in line:
            raise SystemExit(f"{mode} {streams}: {line}")
    if p.wait() != 0:
        raise SystemExit(f"{mode} {streams}: exit {p.returncode}")
    return samples


with open(os.path.join(out, "streams.tsv"), "w") as tsv:
    tsv.write("rep\tstreams\tmode\tbarrier\trss_kib\n")
    for rep in range(1, reps + 1):
        for streams in STREAMS:
            log = os.path.join(out, f"streams-{streams}")
            shutil.rmtree(log, ignore_errors=True)
            for mode in ("append", "recover"):
                for barrier, kib in run(mode, log).items():
                    tsv.write(f"{rep}\t{streams}\t{mode}\t{barrier}\t{kib}\n")
                    print(rep, streams, mode, barrier, kib)
            shutil.rmtree(log)
