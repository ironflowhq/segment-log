#!/bin/bash
# Run from the repository root, with the pinned Rust toolchain.
set -euo pipefail
out="${1:-$PWD/.audit/slice10/macos}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
trap 'printf "exit=%s\n" "$?" > "$out/exit-status.txt"' EXIT
export CARGO_TARGET_DIR="$out/build"
export SEGMENT_LOG_EVIDENCE_DIR="$out/data"
{
    date -u
    sw_vers
    rustc -Vv
    sysctl hw.model hw.memsize hw.physicalcpu hw.logicalcpu machdep.cpu.brand_string
    df -h "$out"
    diskutil info "$(df -P "$out" | awk 'NR==2 {print $1}')" | sed '/UUID/d'
    uptime
} > "$out/environment.txt"
cargo fmt --all --check > "$out/fmt.log" 2>&1
cargo clippy --workspace --all-targets -- -D warnings > "$out/clippy.log" 2>&1
cargo test --workspace > "$out/tests.log" 2>&1
python3 -m unittest discover -s scripts -p 'test_m1_*.py' > "$out/verifier-tests.log" 2>&1
cargo test -p segment-log-core cited_cases_are_defined_and_uncited_cases_are_reported -- --nocapture > "$out/coverage.log" 2>&1
RUSTFLAGS='--cfg fuzzing' cargo check -p segment-log-core > "$out/fuzz.log" 2>&1
cargo test -p segment-log-core --lib acceptance::evidence::real_storage_sync_phases -- --exact --nocapture > "$out/directory.log" 2>&1
