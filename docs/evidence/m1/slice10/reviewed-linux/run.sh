#!/bin/sh
set -eu
task_container="segment-log-m1-parent-$$"
trap 'docker rm -f "$task_container" >/dev/null 2>&1 || true' EXIT HUP INT TERM
out="$PWD/.audit/slice10-parent-linux"
docker run --rm --name "$task_container" --pids-limit 4096 --cap-add SYS_PTRACE --security-opt seccomp=unconfined --mount "type=bind,src=$PWD,dst=/src,readonly" --mount "type=bind,src=$out,dst=/out" -w /src rust:1.99.1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c bash /out/guest.sh > "$out/runner.log" 2>&1
