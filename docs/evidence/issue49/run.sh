#!/usr/bin/env bash
# Run as root on the host, on 2026-11-16. /tmp is ext4 on nvme1n1, one drive, no RAID.
. "$HOME/.cargo/env"
cd /root && rm -rf sl && git clone -q https://github.com/ironflowhq/segment-log.git sl && cd sl
for rev in main ff746473929eccb5398f2b7d4d3fa5ba90f8b10c; do
  git checkout -q $rev; rustup toolchain install >/dev/null 2>&1
  o=/root/m1/out-${rev:0:7}; rm -rf $o /tmp/m1; mkdir -p $o /root/m1/build /tmp/m1/data
  { cat /proc/mdstat; cat /sys/block/nvme*/queue/write_cache; for d in /dev/nvme0 /dev/nvme1; do nvme id-ctrl $d; done; } >$o/host-extra.txt 2>&1
  M1_OUT=$o CARGO_TARGET_DIR=/root/m1/build SEGMENT_LOG_EVIDENCE_DIR=/tmp/m1/data bash scripts/m1-evidence-guest.sh >$o/run.log 2>&1
  echo "$rev exit $?" >>/root/bench-summary
done
