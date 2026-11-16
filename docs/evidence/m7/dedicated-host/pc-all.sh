#!/usr/bin/env bash
export M7_OBSERVER=178.156.201.194:7070
cd /root/segment-log
for e in process-crash guest-crash guest-destroy; do
  bash scripts/m7-power-cut.sh $e 30 >/root/pc-$e.log 2>&1; echo "$e real exit $?" >>/root/pc-summary
  bash scripts/m7-power-cut.sh $e 10 --control >/root/pc-$e-control.log 2>&1; echo "$e control exit $?" >>/root/pc-summary
done
echo done >>/root/pc-summary
