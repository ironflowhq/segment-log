#!/usr/bin/env python3
"""The V9 VOL-CAPACITY sizing record (volume-requirements.md, V9) from a workload profile.

  m7-capacity.py [--history-bytes 50GiB] [--logs 8] [--ratio R] [--avail-ratio A] ...

The defaults are the provisional workload profile (qualification.md, Provisional values).

Prints key=value lines: the planned peak encoded size of one log, its planned allocated size (encoded
times the measured allocation ratio, plus headroom), the alert thresholds of the guest and host
filesystems (peak write rate times response time), and the free bytes each layer needs.

Every width is from docs/architecture/format.md (the only source); each constant names its table.
test_m7_capacity.py fails when format.md's format_version changes, so a format change revisits them.
"""
import argparse
import math
import sys

FORMAT_VERSION = 3
SEGMENT_HEADER = 64  # Segment header
ENVELOPE = 8  # Frame: body_len, crc
BODY_HEADER = 24  # Body header
ENTRY = 16  # Group record entry: stream_len, key_len, payload_len, revision
# Bytes a group frame carries beyond the version 2 group body: version 3's 8-byte append_time.
GROUP_EXTRA = 8
FOOTER_FIXED = 8  # Footer frame: key_filter_position
FOOTER_ENTRY = 28  # Footer entry, before the stream id
LOCATOR = ENVELOPE + BODY_HEADER + 8  # Locator frame: the whole frame is 40 bytes
KEYS_PER_PAGE = 255  # Key index frame: entry slots per page
PAGE = 4096
KEYS_PER_BLOCK = 32  # Key filter frame: B = ceil(n / 32)
BLOCK = 64
FENCE = 8
MAX_PAD = PAGE - 1  # the filter's padding depends on its file position; size for the worst case
MAX_KEYED_PER_SEGMENT = 1_044_225  # Limits
MAX_GROUP_BODY = 8 * 1024 * 1024  # Limits
MAX_FRAME_BODY = 16 * 1024 * 1024  # Limits


def entry_bytes(payload, stream, key):
    return ENTRY + stream + key + payload


def group_frame(records, payload, stream, key, extra=GROUP_EXTRA):
    return ENVELOPE + BODY_HEADER + extra + records * entry_bytes(payload, stream, key)


def records_per_group(group_bytes, payload, stream, key, extra=GROUP_EXTRA):
    """The most profile records whose group frame fits in group_bytes."""
    n = (group_bytes - ENVELOPE - BODY_HEADER - extra) // entry_bytes(payload, stream, key)
    if n < 1:
        raise ValueError("one record does not fit in the group")
    return n


def closing_run(keyed, streams, stream):
    """A roll's closing run: key filter, key index, footer, locator (format.md, sealed segment layout).

    The final segment of a sealed log has a smaller run (footer, seal, locator), so this bounds both.
    """
    pages = math.ceil(keyed / KEYS_PER_PAGE)
    blocks = math.ceil(keyed / KEYS_PER_BLOCK)
    pad = MAX_PAD if keyed else 0
    key_filter = ENVELOPE + BODY_HEADER + FENCE * pages + BLOCK * blocks + pad
    key_index = ENVELOPE + BODY_HEADER + PAGE * pages
    footer = ENVELOPE + BODY_HEADER + FOOTER_FIXED + streams * (FOOTER_ENTRY + stream)
    return key_filter + key_index + footer + LOCATOR


def segment_bytes(groups, per_group, last_group, profile):
    """One closed segment: header, groups (the last may be short), closing run.

    Each record is its own stream unless profile["streams"] caps them: the footer's worst case.
    """
    p = profile
    records = (groups - 1) * per_group + last_group
    keyed = records if p["key"] else 0
    streams = min(records, p["streams"]) if p["streams"] else records
    body = (groups - 1) * group_frame(per_group, p["payload"], p["stream"], p["key"], p["extra"])
    body += group_frame(last_group, p["payload"], p["stream"], p["key"], p["extra"])
    return SEGMENT_HEADER + body + closing_run(keyed, streams, p["stream"])


def groups_per_segment(roll_bytes, frame, per_group, keyed, stream=0, streams=0):
    # The writer rolls before a group once the segment reaches roll_bytes, when the group's keys
    # would pass the keyed-record limit (format.md, Limits), or when its new streams would make the
    # footer body pass the frame body limit (format.md, Footer entry).
    groups = max(1, math.ceil((roll_bytes - SEGMENT_HEADER) / frame))
    if keyed:
        groups = min(groups, MAX_KEYED_PER_SEGMENT // per_group)
    footer_streams = (MAX_FRAME_BODY - BODY_HEADER - FOOTER_FIXED) // (FOOTER_ENTRY + stream)
    if not streams or streams > footer_streams:
        # shortcut: with capped streams this assumes each record brings a new one, which rolls early
        # and so overstates the size; model stream reuse if a profile ever needs the exact figure.
        groups = min(groups, footer_streams // per_group)
    return groups


def encoded_bytes(records, profile):
    """Planned encoded size of a log of `records` profile records; every segment counted closed."""
    p = profile
    per_group = p["per_group"]
    frame = group_frame(per_group, p["payload"], p["stream"], p["key"], p["extra"])
    gps = groups_per_segment(p["roll_bytes"], frame, per_group, p["key"], p["stream"], p["streams"])
    per_segment = gps * per_group
    full, rest = divmod(records, per_segment)
    total = full * segment_bytes(gps, per_group, per_group, p) if full else 0
    if rest:
        groups, last = divmod(rest, per_group)
        if last:
            groups += 1
        else:
            last = per_group
        total += segment_bytes(groups, per_group, last, p)
    return total


def records_for(history, profile):
    """The most records whose encoded log fits in `history` bytes."""
    lo, hi = 0, max(1, history)
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if encoded_bytes(mid, profile) <= history:
            lo = mid
        else:
            hi = mid - 1
    return lo


def size(args):
    profile = {
        "payload": args.payload,
        "stream": args.stream,
        "key": args.key,
        "extra": args.group_extra,
        "roll_bytes": args.roll_bytes,
        "streams": args.streams,
    }
    per_group = args.records_per_group or records_per_group(args.group_bytes, args.payload, args.stream, args.key, args.group_extra)
    if BODY_HEADER + args.group_extra + per_group * entry_bytes(args.payload, args.stream, args.key) > MAX_GROUP_BODY:
        raise ValueError("the group body passes the 8 MiB limit")
    profile["per_group"] = per_group
    records = args.records if args.records is not None else records_for(args.history_bytes, profile)
    encoded = encoded_bytes(records, profile)
    allocated = math.ceil(encoded * args.ratio * (1 + args.headroom))
    guest_threshold = math.ceil(args.rate * args.response)
    host_threshold = args.logs * guest_threshold
    # The guest alert must stay quiet until the history passes its plan, so its free space covers both.
    guest_free = allocated + guest_threshold
    gib = 1 << 30
    image = math.ceil(guest_free / args.avail_ratio / gib) * gib
    # The images are sparse, so each allocates at most its size on the host filesystem.
    host_free = args.logs * image + host_threshold
    out = {
        "format_version": FORMAT_VERSION,
        "group_extra_bytes": args.group_extra,
        "record_entry_bytes": entry_bytes(args.payload, args.stream, args.key),
        "records_per_group": per_group,
        "group_frame_bytes": group_frame(per_group, args.payload, args.stream, args.key, args.group_extra),
        "records_per_log": records,
        "encoded_bytes_per_log": encoded,
        "allocation_ratio": args.ratio,
        "headroom": args.headroom,
        "planned_allocated_bytes_per_log": allocated,
        "peak_write_bytes_per_s_per_log": args.rate,
        "response_s": args.response,
        "guest_alert_threshold_bytes": guest_threshold,
        "guest_free_needed_bytes": guest_free,
        "avail_ratio": args.avail_ratio,
        "image_bytes": image,
        "logs": args.logs,
        "host_alert_threshold_bytes": host_threshold,
        "host_free_needed_bytes": host_free,
    }
    if args.host_free is not None:
        out["host_free_bytes"] = args.host_free
        out["host_fits"] = "yes" if args.host_free >= host_free else "no"
    return out


UNITS = {"": 1, "KiB": 1 << 10, "MiB": 1 << 20, "GiB": 1 << 30, "TiB": 1 << 40, "KB": 10**3, "MB": 10**6, "GB": 10**9, "TB": 10**12}


def bytes_arg(text):
    digits = text.rstrip("KMGTiB")
    unit = text[len(digits) :]
    if unit not in UNITS or not digits.isdigit():
        raise argparse.ArgumentTypeError(f"not a byte count: {text}")
    return int(digits) * UNITS[unit]


def parser():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--payload", type=int, default=256, help="mean payload bytes")
    p.add_argument("--stream", type=int, default=16, help="mean stream id bytes")
    p.add_argument("--key", type=int, default=16, help="mean idempotency key bytes; 0 for unkeyed records")
    p.add_argument("--streams", type=int, default=0, help="distinct streams; 0 means one per record (the footer's worst case)")
    p.add_argument("--group-bytes", type=bytes_arg, default=16 * 1024, help="group frame size that sets records per group")
    p.add_argument("--records-per-group", type=int, default=0, help="mean records per group, in place of --group-bytes")
    p.add_argument("--group-extra", type=int, default=GROUP_EXTRA, help="bytes a format version adds to each group frame")
    p.add_argument("--roll-bytes", type=bytes_arg, default=64 * 1024 * 1024, help="roll_bytes (api.md default)")
    who = p.add_mutually_exclusive_group()
    who.add_argument("--records", type=int, help="planned peak records per log, in place of --history-bytes")
    who.add_argument("--history-bytes", type=bytes_arg, default=50 << 30, help="planned peak encoded history per log")
    p.add_argument("--ratio", type=float, default=1.0, help="measured allocation ratio: du / encoded bytes")
    p.add_argument("--headroom", type=float, default=0.2, help="recorded headroom, as a fraction")
    p.add_argument("--rate", type=bytes_arg, default=200 * 16 * 1024, help="peak write bytes per second per log")
    p.add_argument("--response", type=int, default=3600, help="response time in seconds")
    p.add_argument("--logs", type=int, default=8, help="logs per host")
    p.add_argument("--avail-ratio", type=float, default=1.0, help="measured df avail / image size after mkfs.ext4")
    p.add_argument("--host-free", type=bytes_arg, help="free bytes of the host volume filesystem, to check the fit")
    return p


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        out = size(args)
    except ValueError as e:
        print(f"m7-capacity: {e}", file=sys.stderr)
        return 64
    for k, v in out.items():
        print(f"{k}={v}")
    return 0 if out.get("host_fits", "yes") == "yes" else 1


if __name__ == "__main__":
    sys.exit(main())
