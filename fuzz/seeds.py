"""Writes seed inputs for the fuzz targets from the format v1 fixture.

Each seed starts with the mode byte that the target reads. Mode 0 keeps the bytes as they are.
"""

import pathlib
import struct

FUZZ = pathlib.Path(__file__).parent
FIXTURE = FUZZ.parent / "segment-log-core/tests/fixtures/format-v1"
HEADER_LEN = 64


def frames(segment):
    at = HEADER_LEN
    while at + 8 <= len(segment):
        end = at + 8 + struct.unpack_from("<I", segment, at)[0]
        yield segment[at:end]
        at = end


def main():
    seeds = {"decoder": set(), "recovery": set()}
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
