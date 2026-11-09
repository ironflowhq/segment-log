#!/bin/bash
set -euo pipefail
out="${1:?usage: scripts/m1-compare-codec.sh OUTPUT_DIRECTORY [BASE_SHA]}"
base="${2:-911e809e72fc9c9ef5571f7022c89e55a48d6cb1}"
mkdir -p "$out/before" "$out/after"
git show "$base:segment-log-core/src/codec.rs" > "$out/before/codec.rs"
cp segment-log-core/src/codec.rs "$out/after/codec.rs"
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
        let bytes = &payload[..len];
        out.write_all(&codec::crc32c(bytes).to_le_bytes()).unwrap();
        for key in [None, Some(b"key".as_slice())] {
            let group = codec::Group {
                epoch: u64::MAX,
                base_offset: 7,
                entries: vec![
                    codec::Entry { stream: b"s", key, payload: bytes, revision: 5 },
                    codec::Entry { stream: &[255; 1024], key: None, payload: &[], revision: 0 },
                ],
            };
            out.write_all(&codec::encode(&group).unwrap()).unwrap();
        }
    }
    for epoch in [0, 1, u64::MAX] {
        out.write_all(&codec::encode_header(&codec::Header {
            log_id: [173; 16], base_offset: 7, epoch,
        })).unwrap();
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
printf 'PASS: checksums, group frames, and headers match source %s\n' "$base"
