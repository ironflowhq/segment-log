"""Writes seed inputs for the fuzz targets from the format fixture.

Each seed starts with the mode byte that the target reads. Mode 0 keeps the bytes as they are.
"""

import pathlib
import struct

FUZZ = pathlib.Path(__file__).parent
FIXTURE = FUZZ.parent / "segment-log-core/tests/fixtures/format-v2"
HEADER_LEN = 64


def frames(segment):
    at = HEADER_LEN
    while at + 8 <= len(segment):
        end = at + 8 + struct.unpack_from("<I", segment, at)[0]
        yield segment[at:end]
        at = end


def append(stream, key, payload_byte, length):
    """One submit of one record for the log target: op, record count, stream, key, payload."""
    keyed = [1, key] if key is not None else [0]
    return [0, 0, stream, *keyed, payload_byte, length, 1]


def log_scripts():
    """Scripts for the log target: flags, roll size, simulator seed, then steps (engine/fuzzing.rs)."""
    appends = [b for n in range(12) for b in append(n % 4, n if n % 3 else None, n, 30)]
    reopen, kill, power_loss, torn, seal = [4], [5, 0, 1], [5, 1, 7], [5, 2, 9], [7]
    bodies = [
        appends + reopen + appends + seal,
        appends + power_loss + appends + torn + appends,
        appends + kill + appends + reopen + appends + seal,
    ]
    for flags in (0, 1, 3):
        for body in bodies:
            yield bytes([flags, 4, 1, *body, 0, 200, 33])


def main():
    seeds = {"decoder": set(), "recovery": set(), "log": set(log_scripts())}
    for path in sorted(FIXTURE.glob("*/*.seg")):
        segment = path.read_bytes()
        seeds["recovery"].add(b"\0" + segment[HEADER_LEN:])
        seeds["decoder"].add(b"\0" + segment[:HEADER_LEN])
        seeds["decoder"].update(b"\0" + frame for frame in frames(segment))
    for target, inputs in seeds.items():
        out = FUZZ / "seeds" / target
        out.mkdir(parents=True, exist_ok=True)
        for i, data in enumerate(sorted(inputs)):
            (out / f"{i:03}").write_bytes(data)
        print(f"{target}: {len(inputs)} seeds")


if __name__ == "__main__":
    main()
