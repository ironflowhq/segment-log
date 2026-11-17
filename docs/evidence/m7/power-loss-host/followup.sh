#!/usr/bin/env bash
# After the host-cut session: guest-bench before and after PR #135 on the same box, plus the
# server write-path checks (rehearse, guest-side power cuts) on the merged main.
cd "$(dirname "$0")/.."
until grep -q "^== done" m7-power-loss-out/session.log; do sleep 30; done
export M7_HOST=root@195.201.169.204 M7_OBSERVER=178.156.201.194:7070
on_host() { ssh -o ConnectTimeout=10 -o BatchMode=yes "$M7_HOST" "$@"; }
echo "== baseline guest-bench at $(on_host 'git -C /root/segment-log rev-parse --short HEAD') $(date -u +%T)"
M7_OUT=$PWD/m7-power-loss-out/before scripts/m7-session.sh run guest-bench
echo "== update the host to main $(date -u +%T)"
on_host 'set -e; cd /root/segment-log && git fetch -q && git checkout -q origin/main && git rev-parse --short HEAD
	. $HOME/.cargo/env && bash -c "source scripts/m7/setup.sh" >/root/setup-after.log 2>&1 && echo setup ok'
git fetch -q origin && git checkout -q origin/main -- scripts/m7-session.sh scripts/m7-guest-bench.sh 2>/dev/null || true
echo "== after #135 $(date -u +%T)"
M7_OUT=$PWD/m7-power-loss-out/after scripts/m7-session.sh run rehearse power-cut guest-bench
echo "== followup done $(date -u +%T)"
