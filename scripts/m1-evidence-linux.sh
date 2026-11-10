#!/bin/sh
# Run from the repository root. Only the named disposable container is removed.
set -eu
out="${1:-$PWD/.audit/slice10/linux}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
docker="${DOCKER:-docker}"
image="rust:1.99.1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c"
name="segment-log-m1-evidence-$$"
trap '"$docker" rm -f "$name" >/dev/null 2>&1 || true' EXIT HUP INT TERM
# The container cannot read a worktree's git directory, and it cannot see the host's other work.
{ git rev-parse HEAD; git status --porcelain; } > "$out/source.txt"
load() {
    date -u
    "$docker" stats --no-stream --format '{{.Name}} {{.CPUPerc}}'
    # Names only: a full path would put the host's directories into the record.
    ps -Ao pcpu,comm -r | head -12 | awk '{n = split($NF, part, "/"); print $1, part[n]}'
}
load > "$out/host-load-before.txt"
"$docker" version > "$out/docker-version.txt"
"$docker" info --format 'driver={{.Driver}} driver_status={{json .DriverStatus}} cpus={{.NCPU}} memory={{.MemTotal}} kernel={{.KernelVersion}} os={{.OperatingSystem}} arch={{.Architecture}}' > "$out/docker-info.txt"
status=0
"$docker" run --rm --name "$name" --pids-limit 4096 --cap-add SYS_PTRACE --security-opt seccomp=unconfined \
    --mount "type=bind,src=$PWD,dst=/src,readonly" \
    --mount "type=bind,src=$out,dst=/out" \
    -e "M1_PROFILE_ONLY=${2:-0}" -w /src "$image" bash /src/scripts/m1-evidence-guest.sh > "$out/runner.log" 2>&1 || status=$?
load > "$out/host-load-after.txt"
exit "$status"
