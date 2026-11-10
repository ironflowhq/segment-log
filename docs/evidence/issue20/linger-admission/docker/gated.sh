#!/bin/bash
# Host-side runner. Same procedure as scripts/m1-compare-latency.sh (settings, warm-ups,
# alternation, /usr/bin/time -v), plus a gate: before each run it waits until the other
# containers on the Docker guest are nearly idle, because they share its processors and disk.
set -uo pipefail
C=${CONTAINER:-seglog-m1gate}
test=acceptance::evidence::measured_appends_survive_reopen
others() {  # total CPU percent of every other container
  docker stats --no-stream --format '{{.Name}} {{.CPUPerc}}' | awk -v c="$C" '$1 != c {gsub("%","",$2); s+=$2} END {printf "%.1f\n", s}'
}
host() {    # total CPU percent of every host process except the guest itself
  ps -Ao pcpu,comm | awk 'NR > 1 && $2 !~ /Virtualization/ {s += $1} END {printf "%.0f\n", s}'
}
gate() {    # prints the last samples; waits at most GATE_TRIES rounds for two quiet rounds in a row
  local quiet=0 tries=0 now here
  while :; do
    now=$(others); here=$(host)
    if awk -v x="$now" -v h="$here" -v limit="${HOST_LIMIT:-150}" 'BEGIN {exit !(x < 10 && h < limit)}'; then quiet=$((quiet+1)); else quiet=0; fi
    tries=$((tries+1))
    if [ $quiet -ge 2 ] || [ $tries -ge ${GATE_TRIES:-60} ]; then echo "$now host_cpu_percent_before=$here quiet=$quiet tries=$tries"; return; fi
    sleep 1
  done
}
bench() {   # bench OUTDIR LABEL BINARY [ENV=VALUE...]
  local out=$1 label=$2 bin=$3; shift 3
  local envs=(-e SEGMENT_LOG_EVIDENCE_DIR=/data/bench -e SEGMENT_LOG_BENCH_CALLERS=1000 -e SEGMENT_LOG_BENCH_ROUNDS=1000 -e SEGMENT_LOG_DEVICE_ROUNDS=1000)
  for e in "$@"; do envs+=(-e "$e"); done
  echo "others_cpu_percent_before=$(gate)" > "$HOSTOUT/$out/others-$label.txt"
  docker exec "${envs[@]}" $C bash -c "uptime > /out/$out/load-$label.txt; /usr/bin/time -v $bin --exact $test --nocapture > /out/$out/$label.log 2>&1" || echo "FAILED $out $label"
  echo "others_cpu_percent_after=$(others) host_cpu_percent_after=$(host)" >> "$HOSTOUT/$out/others-$label.txt"
}
compare() { # compare OUTDIR BEFORE_VARIANT AFTER_VARIANT [ENV=VALUE...]
  local out=$1 before=$2 after=$3; shift 3
  mkdir -p "$HOSTOUT/$out"
  docker exec $C bash -c "sha256sum /build/bin-$before /build/bin-$after > /out/$out/binaries.sha256"
  for side in before after; do
    local v=$before; [ $side = after ] && v=$after
    docker exec -e SEGMENT_LOG_EVIDENCE_DIR=/data/bench -e SEGMENT_LOG_BENCH_CALLERS=1000 -e SEGMENT_LOG_BENCH_ROUNDS=20 -e SEGMENT_LOG_DEVICE_ROUNDS=1000 $C \
      bash -c "/build/bin-$v --exact $test --nocapture > /out/$out/warmup-$side.log 2>&1"
  done
  for run in 1 2 3 4 5; do
    bench $out before-$run /build/bin-$before "$@"
    bench $out after-$run /build/bin-$after "$@"
    echo "$out pair $run: $(cat $HOSTOUT/$out/others-before-$run.txt $HOSTOUT/$out/others-after-$run.txt | tr '\n' ' ')"
  done
}
sweep() {   # sweep OUTDIR VARIANT... : five rounds, one run of each variant per round
  local out=$1; shift
  mkdir -p "$HOSTOUT/$out"
  for run in 1 2 3 4 5; do for v in "$@"; do
    bench $out $v-$run /build/bin-$v
  done; echo "$out round $run done"; done
}
probe() {   # probe OUTFILE COMMAND : gate, run the command inside the container, record the other containers
  local rel=$1; shift
  mkdir -p "$(dirname "$HOSTOUT/$rel")"
  echo "others_cpu_percent_before=$(gate)" > "$HOSTOUT/$rel.others"
  docker exec $C bash -c "$*" > "$HOSTOUT/$rel" 2>&1 || echo "FAILED $rel"
  echo "others_cpu_percent_after=$(others)" >> "$HOSTOUT/$rel.others"
}
HOSTOUT="$(cd "$(dirname "$0")" && pwd)/out"
"$@"
