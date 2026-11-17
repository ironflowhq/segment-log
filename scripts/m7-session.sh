#!/usr/bin/env bash
# The M7 stage 2 session (issue 112) in one place, run on the controller (the Mac). Each step's
# output goes to $M7_OUT/<step>.log and its exit status to $M7_OUT/results.tsv; a failing step does
# not stop the next one, so one session yields every result. The qualification record
# (docs/evidence/m7/qualification.md) cites these logs.
#
#   M7_HOST=root@<server ip> scripts/m7-session.sh bootstrap        from the Hetzner rescue system
#   M7_HOST=... M7_OBSERVER=<ip>:7070 M7_ROBOT_CURLRC=<file> M7_ROBOT_SERVER=<n> \
#     scripts/m7-session.sh run [step...]                            all steps, or the named ones
#   M7_HOST=... scripts/m7-session.sh collect                       host facts and the #49 capture
#
# The observer is a separate machine (memory/hetzner-m7-observer.md describes how it is built).
set -euo pipefail
cmd=${1:?usage: m7-session.sh bootstrap|run|collect [step...]}
shift
: "${M7_HOST:?set M7_HOST to root@<server ip>}"
here=$(cd "$(dirname "$0")" && pwd)
out=${M7_OUT:-$here/../m7-session-out}
mkdir -p "$out"
repo=/root/segment-log
on_host() { ssh -o ConnectTimeout=10 -o BatchMode=yes -o ServerAliveInterval=5 -o StrictHostKeyChecking=accept-new "$M7_HOST" "$@"; }
wait_up() {
	local deadline=$((SECONDS + 900))
	until on_host true 2>/dev/null; do
		[ $SECONDS -lt $deadline ] || { echo "the host did not come back" >&2; exit 1; }
		sleep 5
	done
}

bootstrap() {
	# Debian 12 on the first NVMe drive without RAID; the second drive, whole, holds the volumes.
	on_host 'TERM=xterm /root/.oldroot/nfs/install/installimage -a -n m7 -r no -d nvme0n1 \
		-p /boot:ext4:1G,/:ext4:all -i /root/.oldroot/nfs/images/Debian-1215-bookworm-amd64-base.tar.zst \
		-K /root/.ssh/authorized_keys' | tail -n 3
	on_host reboot || true
	ssh-keygen -R "${M7_HOST#*@}" >/dev/null 2>&1 || true
	sleep 30
	wait_up
	# shellcheck disable=SC2016 # expands on the host
	on_host 'set -e
		mkfs.ext4 -q -F -L m7log /dev/nvme1n1
		echo "LABEL=m7log /tmp ext4 defaults,noatime 0 2" >>/etc/fstab
		mount /tmp && chmod 1777 /tmp
		apt-get update -qq
		DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential curl git squashfs-tools \
			musl-tools nvme-cli fio blktrace dmsetup sudo time python3 >/dev/null
		curl -fsSL https://go.dev/dl/go1.27.3.linux-amd64.tar.gz | tar -C /usr/local -xz
		curl -fsSL https://sh.rustup.rs | sh -s -- -y -q --default-toolchain none >/dev/null 2>&1
		git clone -q https://github.com/ironflowx/segment-log.git '"$repo"'
		cd '"$repo"' && . "$HOME/.cargo/env" && rustup toolchain install >/dev/null 2>&1
		bash -c "source scripts/m7/setup.sh" >/root/setup.log 2>&1
		echo bootstrapped: $(git -C '"$repo"' rev-parse --short HEAD)'
}

# step <name> <command...>: runs one step, logs it, records its status, never stops the session.
step() {
	local name=$1 rc=0
	shift
	echo "== $name"
	"$@" >"$out/$name.log" 2>&1 || rc=$?
	printf '%s\t%s\t%s\n' "$name" "$rc" "$(date -u +%FT%TZ)" >>"$out/results.tsv"
	echo "   exit $rc: $(tail -n 1 "$out/$name.log")"
}
host_step() { # a script that runs on the host, as root, from the repo
	local name=$1
	shift
	step "$name" on_host "cd $repo && M7_OBSERVER=$M7_OBSERVER M7_CACHE=/root/.cache/segment-log-m7 $*"
}

run() {
	: "${M7_OBSERVER:?set M7_OBSERVER to <observer ip>:7070}"
	local all=(rehearse power-cut drift latency metadata enospc guest-bench workloads host-cut recovery-cut)
	local steps=("${@:-${all[@]}}")
	for s in "${steps[@]}"; do
		case $s in
		rehearse) host_step rehearse bash scripts/m7-rehearse.sh ;;
		power-cut)
			for e in process-crash guest-crash guest-destroy; do
				host_step "pc-$e" bash scripts/m7-power-cut.sh "$e" 30
				host_step "pc-$e-control" bash scripts/m7-power-cut.sh "$e" 10 --control
			done
			;;
		drift) host_step drift bash scripts/m7-drift.sh ;;
		latency)
			# Planned density and group rate are provisional (issue 112): 8 guests, 200 groups/s each.
			host_step latency-alone bash scripts/m7-flush-latency.sh 1 0 60
			host_step latency-density bash scripts/m7-flush-latency.sh 8 200 60
			host_step latency-saturation bash scripts/m7-flush-latency.sh 8 0 60
			;;
		metadata)
			host_step metadata bash scripts/m7-metadata-error.sh
			host_step metadata-control bash scripts/m7-metadata-error.sh --control
			host_step metadata-panic bash scripts/m7-metadata-error.sh --panic
			;;
		guest-bench)
			host_step guest-bench bash scripts/m7-guest-bench.sh /root/m7-guest-bench
			scp -rq "$M7_HOST:/root/m7-guest-bench" "$out/guest-bench" || true
			;;
		workloads)
			host_step workloads bash scripts/m7-workloads.sh /root/m7-workloads
			scp -rq "$M7_HOST:/root/m7-workloads" "$out/workloads" || true
			;;
		enospc)
			host_step enospc-guest bash scripts/m7-enospc.sh guest
			host_step enospc-host bash scripts/m7-enospc.sh host
			;;
		host-cut)
			: "${M7_ROBOT_CURLRC:?set M7_ROBOT_CURLRC}" "${M7_ROBOT_SERVER:?set M7_ROBOT_SERVER}"
			for e in host-kernel-crash host-reset host-power-loss; do
				step "hc-$e" "$here/m7-host-cut.sh" "$e" 30
				step "hc-$e-control" "$here/m7-host-cut.sh" "$e" 30 --control
			done
			;;
		recovery-cut)
			for f in clean sealed header; do
				M7_OUT=$out step "v2-$f" "$here/m7-recovery-cut.sh" "$f" 10
			done
			M7_OUT=$out step v2-control "$here/m7-recovery-cut.sh" clean 10 --control
			;;
		*) echo "unknown step $s" >&2; exit 64 ;;
		esac
	done
}

collect() {
	# shellcheck disable=SC2016 # expands on the host
	on_host 'uname -a; lscpu | grep "Model name"; free -g | head -n 2; findmnt -T /tmp; cat /proc/mdstat
		for d in /dev/nvme0 /dev/nvme1; do nvme id-ctrl "$d" | grep -E "^(mn|fr|vwc|awupf|oncs) "; done
		nvme id-ns /dev/nvme1n1 | grep -E "^(nawupf|lbaf)"; cat /sys/block/nvme1n1/queue/write_cache
		/root/.cache/segment-log-m7/firecracker --version | head -n 1' >"$out/host.txt" 2>&1
	# The #49 capture on the qualified host, with the per-roll writer times (ADR 0061).
	step m1-capture on_host "cd $repo && . \$HOME/.cargo/env && mkdir -p /root/m1/out /tmp/m1/data &&
		M1_OUT=/root/m1/out CARGO_TARGET_DIR=/root/m1/build SEGMENT_LOG_EVIDENCE_DIR=/tmp/m1/data \
		bash scripts/m1-evidence-guest.sh"
	scp -rq "$M7_HOST:/root/m1/out" "$out/m1-capture" || true
}

case $cmd in
bootstrap) bootstrap ;;
run) run "$@" ;;
collect) collect ;;
*) echo "unknown command $cmd" >&2; exit 64 ;;
esac
