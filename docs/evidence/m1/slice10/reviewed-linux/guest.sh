#!/bin/bash
set -euo pipefail
trap 'printf "exit=%s\n" "$?" > /out/exit-status.txt' EXIT
export CARGO_TARGET_DIR=/build
apt-get update
apt-get install -y --no-install-recommends strace python3
rustc -Vv > /out/toolchain.txt
uname -a >> /out/toolchain.txt
cargo fmt --all --check > /out/fmt.log 2>&1
cargo clippy --workspace --all-targets -- -D warnings > /out/clippy.log 2>&1
cargo test --workspace > /out/tests.log 2>&1
python3 -B -m unittest discover -s scripts -p 'test_m1_*.py' > /out/verifier-tests.log 2>&1
RUSTFLAGS='--cfg fuzzing' cargo check -p segment-log-core > /out/fuzz.log 2>&1
cargo test -p segment-log-core --lib --release --no-run --message-format=json > /out/build.jsonl
binary="$(python3 -c 'import json; print(next(x["executable"] for x in map(json.loads,open("/out/build.jsonl")) if x.get("executable")))')"
strace -f -tt -T -yy -s 256 -e trace=fdatasync,fsync,write -o /out/syscalls.trace "$binary" --exact acceptance::evidence::real_storage_sync_phases --nocapture > /out/sync-phases.log 2>&1
python3 scripts/m1-verify-syscalls.py /out/syscalls.trace > /out/syscalls-verified.txt
