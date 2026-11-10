#!/bin/sh
# Run from the repository root on the Docker host, with a new output directory.
# Only the container named below is created and removed. The container is privileged and shares
# the host's process ids, because the kernel trace names processes by those ids.
set -eu
out="${1:-$PWD/.audit/issue48/run}"
mkdir -p "$(dirname "$out")"
mkdir "$out"
out="$(cd "$out" && pwd)"
docker="${DOCKER:-docker}"
image="rust:1.99.1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c"
name="seglog-m1-exact"
bin=/build/release/examples
driver=scripts/m1-exact-peaks.py
histories="--history 16k=16384,0,1,256,32 --history 16k-keyed=16384,16384,1,256,32 \
    --history 1m=1000000,0,1000,1000,32 --history 1m-keyed=1000000,1000000,1000,1000,32"

{ sw_vers; sysctl -n machdep.cpu.brand_string hw.ncpu hw.memsize; } > "$out/host.txt" 2>&1 || true
{ git rev-parse HEAD; git status --short; } > "$out/sources.txt"
"$docker" info --format 'driver={{.Driver}} cpus={{.NCPU}} memory={{.MemTotal}} kernel={{.KernelVersion}} os={{.OperatingSystem}} arch={{.Architecture}}' > "$out/docker-info.txt"
# The build and the logs stay on the container's own filesystem. Only /out is a bind mount.
"$docker" run -d --name "$name" --privileged --pid=host \
    --mount "type=bind,src=$PWD,dst=/src,readonly" --mount "type=bind,src=$out,dst=/out" \
    -e CARGO_TARGET_DIR=/build -w /src "$image" sleep infinity > /dev/null
trap '"$docker" rm -f "$name" > /dev/null' EXIT HUP INT TERM

"$docker" exec -i "$name" bash -euo pipefail -s <<GUEST
mountpoint -q /sys/kernel/tracing || mount -t tracefs nodev /sys/kernel/tracing
{
    echo "image=$image"
    date -u
    uname -a
    nproc
    echo "page size \$(getconf PAGESIZE)"
    echo "transparent huge pages \$(cat /sys/kernel/mm/transparent_hugepage/enabled)"
    ldd --version | head -1
    python3 -V
    rustc -Vv
    cargo -V
    free -b
    cat /sys/kernel/tracing/events/kmem/rss_stat/format
    mkdir -p /data /build
    df -T /data /build /out
    lscpu
} > /out/environment.txt 2>&1
cargo build --release -p segment-log-core --example resident_probe --example owner_probe > /out/build.log 2>&1
sha256sum $bin/resident_probe $bin/owner_probe $driver scripts/m1-linux-phase-peaks.py > /out/binaries.sha256
python3 -B -m unittest discover -s scripts -p 'test_m1_*.py' > /out/python-tests.log 2>&1
python3 -B docs/evidence/issue48/faults.py > /out/faults.log 2>&1
GUEST

cpu=$("$docker" exec "$name" sh -c 'echo $(($(nproc) - 1))')
"$docker" stats --no-stream > "$out/docker-stats-before.txt"
"$docker" exec "$name" python3 -B $driver peaks $bin/resident_probe --data /data/peaks --output /out \
    --traces /out/traces --cpu "$cpu" --repetitions 3 $histories > "$out/peaks.log" 2>&1
"$docker" exec "$name" python3 -B $driver owners $bin/owner_probe --data /data/owners --output /out \
    --repetitions 3 $histories > "$out/owners.log" 2>&1
"$docker" stats --no-stream > "$out/docker-stats-after.txt"

"$docker" exec "$name" sh -c "python3 -B $driver summary /out --csv /out/readings.csv > /out/summary.md"
# Pack inside the container, so that the archives carry no host account name.
"$docker" exec "$name" tar -C /out -czf /out/raw.tar.gz peaks owners
"$docker" exec "$name" sh -c "cd /out && sha256sum traces/*.trace.gz > traces.sha256"
