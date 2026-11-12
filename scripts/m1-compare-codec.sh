#!/bin/bash
set -euo pipefail
out="${1:?usage: scripts/m1-compare-codec.sh OUTPUT_DIRECTORY [BASE_SHA]}"
base="${2:-911e809e72fc9c9ef5571f7022c89e55a48d6cb1}"
mkdir -p "$out/before" "$out/after"
git show "$base:segment-log-core/src/codec.rs" > "$out/before/codec.rs"
cp segment-log-core/src/codec.rs "$out/after/codec.rs"
if [ -d segment-log-core/src/codec ]; then
    cp -R segment-log-core/src/codec "$out/after/codec"
fi
# Frames and headers changed on purpose in format v1 (ADR 0049), so only the checksum is comparable.
cat > "$out/fixture.rs" <<'RS'
mod codec;
use std::io::{self, Write};
fn main() {
    let mut out = io::stdout().lock();
    for word in 0..=u16::MAX {
        out.write_all(&codec::crc32c(&word.to_le_bytes()).to_le_bytes()).unwrap();
    }
    let payload: Vec<u8> = (0..65537).map(|n| ((n * 97 + n / 251) % 256) as u8).collect();
    for len in [0, 1, 7, 8, 255, 256, 257, 4095, 4096, 8193, 65537] {
        out.write_all(&codec::crc32c(&payload[..len]).to_le_bytes()).unwrap();
    }
}
RS
for side in before after; do
    cp "$out/fixture.rs" "$out/$side/main.rs"
    rustc --edition=2024 -O "$out/$side/main.rs" -o "$out/$side/fixture" 2> "$out/$side/build.log"
    "$out/$side/fixture" > "$out/$side/bytes.bin"
done
cmp "$out/before/bytes.bin" "$out/after/bytes.bin"
wc -c "$out/before/bytes.bin"
shasum -a 256 "$out/before/bytes.bin" "$out/after/bytes.bin"
printf 'PASS: checksums match source %s\n' "$base"
