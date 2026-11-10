#!/bin/bash
# Captures the M3 roll syscall evidence on Linux. Run from the repository root with strace installed.
# Usage: scripts/m3-evidence.sh OUT_DIR
set -euo pipefail
out="${1:?usage: m3-evidence.sh OUT_DIR}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
trap 'printf "exit=%s\n" "$?" > "$out/exit-status.txt"' EXIT
{
    date -u
    # A container cannot read a worktree's git directory.
    git rev-parse HEAD 2>/dev/null || echo "source commit: unknown"
    rustc -Vv
    cargo -V
    strace -V | head -1
    python3 --version
    uname -a
    cat /etc/os-release
} > "$out/environment.txt"
test=acceptance::evidence::real_storage_roll_phases
cargo test -p segment-log-core --lib --release --no-run --message-format=json > "$out/build.jsonl"
binary="$(python3 -c 'import json,sys; print(next(x["executable"] for x in map(json.loads,open(sys.argv[1])) if x.get("executable")))' "$out/build.jsonl")"
sha256sum "$binary" > "$out/binary.sha256"
python3 -B -m unittest discover -s scripts -p 'test_m3_*.py' > "$out/verifier-tests.log" 2>&1
strace -f -tt -T -yy -s 256 -e trace=openat,write,fdatasync,fsync -o "$out/syscalls.trace" \
    "$binary" --exact "$test" --nocapture > "$out/roll-phases.log" 2>&1
python3 scripts/m3-verify-roll-syscalls.py "$out/syscalls.trace" > "$out/syscalls-verified.txt"
