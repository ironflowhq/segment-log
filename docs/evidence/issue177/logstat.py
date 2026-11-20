#!/usr/bin/env python3
"""Issue 177: facts about a sealed log directory that the read times depend on.

    logstat.py frames DIR     group frames per segment, from the frame envelopes (format.md)
    logstat.py resident DIR   the log's bytes, and how many of them the page cache holds

`resident` maps each segment file and asks mincore(2) which pages are resident. It reads no
page, so it does not change what it measures.
"""
import ctypes
import ctypes.util
import mmap
import os
import statistics
import struct
import sys


def segments(d):
    return sorted(os.path.join(d, n) for n in os.listdir(d) if n.endswith(".seg"))


def frames(d):
    counts = []
    for path in segments(d):
        n = 0
        with open(path, "rb") as f:
            at = 64
            while True:
                f.seek(at)
                prefix = f.read(9)
                if len(prefix) < 9 or prefix[8] != 1:
                    break
                n += 1
                at += 8 + struct.unpack_from("<I", prefix)[0]
        counts.append(n)
    print(
        f"FRAMES segments={len(counts)} total={sum(counts)} "
        f"median={statistics.median(counts):.0f} min={min(counts)} max={max(counts)}"
    )


def resident(d):
    libc = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
    libc.mmap.restype = ctypes.c_void_p
    libc.mmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_long]
    libc.mincore.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_char_p]
    libc.munmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    page = os.sysconf("SC_PAGE_SIZE")
    total = held = 0
    for path in segments(d):
        size = os.path.getsize(path)
        if size == 0:
            continue
        fd = os.open(path, os.O_RDONLY)
        try:
            addr = libc.mmap(None, size, mmap.PROT_READ, mmap.MAP_SHARED, fd, 0)
            if addr in (None, ctypes.c_void_p(-1).value):
                raise OSError(ctypes.get_errno(), "mmap " + path)
            pages = (size + page - 1) // page
            vec = ctypes.create_string_buffer(pages)
            if libc.mincore(addr, size, vec) != 0:
                raise OSError(ctypes.get_errno(), "mincore " + path)
            held += sum(b & 1 for b in vec.raw) * page
            libc.munmap(addr, size)
        finally:
            os.close(fd)
        total += size
    try:
        ram = os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
    except (ValueError, OSError):
        import subprocess

        ram = int(subprocess.check_output(["sysctl", "-n", "hw.memsize"]))
    print(
        f"RESIDENT log_bytes={total} ram_bytes={ram} log_over_ram={total / ram:.2f} "
        f"resident_bytes={min(held, total)} fraction={min(held, total) / total:.2f}"
    )


if __name__ == "__main__":
    {"frames": frames, "resident": resident}[sys.argv[1]](sys.argv[2])
