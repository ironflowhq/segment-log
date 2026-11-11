#!/bin/bash
# Gate each run on host load, then run. Usage: run.sh <label> <mode> <page>
D=$(cd "$(dirname "$0")" && pwd)
for i in $(seq 1 120); do
  l=$(sysctl -n vm.loadavg | awk '{print $2}')
  if awk "BEGIN{exit !($l < 3.0)}"; then break; fi
  sleep 5
done
echo "LOAD_START $(sysctl -n vm.loadavg) $(date +%T)"
"$D/target/release/m4-proto" "$2" "$D/data" "$3"
echo "LOAD_END $(sysctl -n vm.loadavg) $(date +%T)"
