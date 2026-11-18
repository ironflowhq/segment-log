#!/usr/bin/env bash
# Issue 154 rerun. Run as root on an EPYC 7401P auction box after `scripts/m7-session.sh bootstrap`
# (Debian 12, the second NVMe as ext4 on /tmp, rustup installed).
#
#   main      both fixes: the active key map is a HashMap (#163), and the benchmark builds the
#             log filter off the writer as `SegmentLog::open` does (#164)
#   ab86546   #163 only, so the benchmark still builds the log filter on the writer
#   386695f   the commit of the recorded capture (docs/evidence/m7/requalified-host/m1-capture)
#
# Each commit gets the unchanged M1 capture. Then main gets 3 more paced runs with the pipeline
# trace on; the trace adds work to every request, so those runs are kept out of the summary.
set -uo pipefail
. "$HOME/.cargo/env"
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq numactl >/dev/null
revs=${*:-main ab86546 386695f}
cd /root && rm -rf sl && git clone -q https://github.com/ironflowx/segment-log.git sl && cd sl
main=$(git rev-parse --short main)
for rev in $revs; do
  git checkout -q "$rev"; rustup toolchain install >/dev/null 2>&1
  o=/root/issue154/$(git rev-parse --short HEAD); rm -rf "$o" /tmp/m1; mkdir -p "$o" /tmp/m1/data
  {
    lscpu; numactl -H 2>/dev/null || true
    cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo "no cpufreq governor"
    cat /sys/block/nvme*/queue/write_cache; for d in /dev/nvme0 /dev/nvme1; do nvme id-ctrl "$d" | grep -E "^(mn|fr|vwc) "; done
    findmnt /tmp
  } > "$o/host-extra.txt" 2>&1
  M1_OUT=$o CARGO_TARGET_DIR=/root/issue154-build SEGMENT_LOG_EVIDENCE_DIR=/tmp/m1/data \
    bash scripts/m1-evidence-guest.sh > "$o/run.log" 2>&1
  echo "$rev $(git rev-parse --short HEAD) capture exit $?" >> /root/issue154/summary.txt
  if [ "$(git rev-parse --short HEAD)" = "$main" ]; then
    binary=$(python3 -c 'import json,sys; print(next(x["executable"] for x in map(json.loads,open(sys.argv[1])) if x.get("executable")))' "$o/build.jsonl")
    for run in 1 2 3; do
      t=$o/traced-$run; mkdir -p "$t"
      uptime > "$t/load.txt"; python3 scripts/m1-quiet.py "$t/load.txt"
      SEGMENT_LOG_BENCH_CALLERS=1000 SEGMENT_LOG_BENCH_ROUNDS=1000 SEGMENT_LOG_DEVICE_ROUNDS=1000 \
        SEGMENT_LOG_BENCH_INTERVAL_US=20000 SEGMENT_LOG_PIPELINE_OUT=$t/t.tsv \
        "$binary" --exact acceptance::evidence::measured_appends_survive_reopen --nocapture > "$t/log" 2>&1
      python3 docs/evidence/issue154/stall.py "$t" > "$t/stall.txt" 2>&1
      rm -f "$t/t.tsv"  # 1M rows; stall.txt, t.groups.tsv and t.events.tsv keep what the record needs
    done
    echo "$rev traced runs done" >> /root/issue154/summary.txt
  fi
done
echo "all done" >> /root/issue154/summary.txt
