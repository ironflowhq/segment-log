"""Cold 4 KiB page reads at aligned and unaligned offsets, as a key index lookup reads them.

A lookup in a sealed segment opens the file, reads one 4,096-byte key index page at its file
position, and closes the file (`storage/real.rs`). Format version 1 puts no key index page on a
4 KiB boundary, so each read spans two OS pages. This measures what that costs on a cold page cache.

Run mode reads `--reads` distinct random pages of an existing file and prints one TSV row:
variant, trial, reads, p50_ns, p90_ns, p99_ns, mean_ns, device_reads, device_kib. Drop the page
cache before each run. Summarize mode reads those rows and prints the median of each column over
the trials of each variant.
"""

import argparse
import os
import random
import statistics
import sys
import time

PAGE = 4096


def quantile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * fraction))]


def device_counters(device):
    """Reads completed and sectors read, from /sys/class/block/<device>/stat."""
    with open(f"/sys/class/block/{device}/stat") as stat:
        fields = stat.read().split()
    return int(fields[0]), int(fields[2])


def offsets(size, reads, seed, aligned):
    rng = random.Random(seed)
    pages = rng.sample(range(size // PAGE - 1), reads)
    return [page * PAGE + (0 if aligned else rng.randrange(1, PAGE)) for page in pages]


def run(args):
    advice = os.POSIX_FADV_RANDOM if args.advice == "random" else os.POSIX_FADV_NORMAL
    size = os.path.getsize(args.file)
    times = []
    before = device_counters(args.device)
    for offset in offsets(size, args.reads, args.seed, args.alignment == "aligned"):
        start = time.perf_counter_ns()
        fd = os.open(args.file, os.O_RDONLY)
        os.posix_fadvise(fd, 0, 0, advice)
        data = os.pread(fd, PAGE, offset)
        os.close(fd)
        times.append(time.perf_counter_ns() - start)
        assert len(data) == PAGE
    after = device_counters(args.device)
    variant = f"{args.alignment}-{args.advice}"
    row = [variant, args.trial, len(times), quantile(times, 0.5), quantile(times, 0.9),
           quantile(times, 0.99), int(statistics.mean(times)), after[0] - before[0],
           (after[1] - before[1]) // 2]
    print("\t".join(map(str, row)))


def summarize(lines):
    """Median of each numeric column over the trials of each variant, in first-seen order."""
    columns = ["p50_ns", "p90_ns", "p99_ns", "mean_ns", "device_reads", "device_kib"]
    by_variant = {}
    for line in lines:
        variant, _trial, _reads, *values = line.split("\t")
        by_variant.setdefault(variant, []).append([int(v) for v in values])
    out = ["\t".join(["variant", "trials", *columns])]
    for variant, rows in by_variant.items():
        medians = [int(statistics.median(column)) for column in zip(*rows)]
        out.append("\t".join(map(str, [variant, len(rows), *medians])))
    return out


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    r = sub.add_parser("run")
    r.add_argument("--file", required=True)
    r.add_argument("--device", required=True, help="block device name under /sys/class/block")
    r.add_argument("--alignment", choices=["aligned", "unaligned"], required=True)
    r.add_argument("--advice", choices=["normal", "random"], required=True)
    r.add_argument("--reads", type=int, default=2000)
    r.add_argument("--seed", type=int, required=True)
    r.add_argument("--trial", type=int, required=True)
    sub.add_parser("summarize")
    args = parser.parse_args()
    if args.command == "run":
        run(args)
    else:
        print("\n".join(summarize(sys.stdin.read().splitlines())))


if __name__ == "__main__":
    main()
