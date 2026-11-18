#!/usr/bin/env bash
# The M7 requalification: every step on one host, then collect.
cd "$(dirname "$0")/.."
export M7_HOST=root@168.119.4.92 M7_OBSERVER=178.156.201.194:7070 M7_OUT=$PWD/m7-requal-out M7_ROBOT_SERVER=3115170
export M7_ROBOT_CURLRC=<robot curlrc>
echo "== run $(date -u +%FT%TZ)"
scripts/m7-session.sh run
echo "== collect $(date -u +%FT%TZ)"
scripts/m7-session.sh collect
echo "== session done $(date -u +%FT%TZ)"
