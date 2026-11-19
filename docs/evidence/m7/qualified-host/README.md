# M7 stage 2 session on the qualified host

The raw output of `scripts/m7-session.sh` for the [qualification record](../qualification.md), run on 2026-11-17 ([issue 112](https://github.com/ironflowx/segment-log/issues/112)). Each step wrote `<step>.log`; [results.tsv](https://github.com/ironflowx/segment-log/blob/962a34a7d7b129b8e50a08458a966d2f17379760/docs/evidence/m7/qualified-host/results.tsv) has each step's exit status and its end time in UTC. The record reads these files; this page describes the host and what the files do not show.

## Host

A Hetzner server-auction machine, #3114789, used on 2026-11-17 and cancelled the same day.

- Intel Xeon E-2176G (6 cores, 12 threads), 64 GB. Debian 12, kernel `6.1.0-54-amd64`, installed by `m7-session.sh bootstrap` on `nvme0n1` ([host.txt](https://github.com/ironflowx/segment-log/blob/962a34a7d7b129b8e50a08458a966d2f17379760/docs/evidence/m7/qualified-host/host.txt)).
- Two Samsung PM983 `MZQLB960HAJR-00007` U.2 drives, firmware `EDA5502Q`, about 56,280 power-on hours each ([smart.txt](https://github.com/ironflowx/segment-log/blob/962a34a7d7b129b8e50a08458a966d2f17379760/docs/evidence/m7/qualified-host/smart.txt)). `vwc 0`: they report no volatile write cache, so the kernel shows `write through` and sends them no flush. `awupf 7`, 512-byte blocks in use.
- The log volumes are raw images on ext4 on `nvme1n1`, whole device, `rw,noatime`, no RAID.
- Firecracker v1.18.0 and the guest artifacts from `scripts/m7/setup.sh` at `df23d9b`.
- The observer ran on a Hetzner Cloud server in Ashburn, separate from the host and the controller, and was deleted after the session. Its store is not committed: every `check` line in the logs already compares the volume against it.

## Host reset is not a power cut

The Robot web service offers this server only `sw`, `hw` and `man` resets, not the `power` and `power_long` button presses that `host-power-loss` needs ([hc-host-power-loss.log](https://github.com/ironflowx/segment-log/blob/962a34a7d7b129b8e50a08458a966d2f17379760/docs/evidence/m7/qualified-host/hc-host-power-loss.log)). The session therefore ran `host-reset` (`hw`, the reset button) as its own event.

The drives' SMART counters show that a reset leaves them powered. After the session both drives report `power_cycles 21` over their whole life and `unsafe_shutdowns 0` ([smart.txt](https://github.com/ironflowx/segment-log/blob/962a34a7d7b129b8e50a08458a966d2f17379760/docs/evidence/m7/qualified-host/smart.txt)). The host booted 94 times since the install ([boots.txt](https://github.com/ironflowx/segment-log/blob/962a34a7d7b129b8e50a08458a966d2f17379760/docs/evidence/m7/qualified-host/boots.txt)), 31 of them by `hw` reset: if a reset removed power, `power_cycles` would be at least 31.

So `host-reset` is a host reboot without a kernel shutdown, with the drives powered: as strong as a host kernel crash for the page cache, and no evidence for power loss.

## Files

- `rehearse.log`: V1 flush configuration, V4, V5 and V6.
- `pc-*.log`: guest-side power cuts; `hc-*.log`: host kernel crash, host reset and the refused host power loss. Each has a `-control` pair.
- `v2-*.log` and the `v2-*` directories: recovery cuts, one stream and one check log per cycle.
- `latency-*.log`, `metadata*.log`, `enospc-*.log`, `drift.log`.
- `guest-bench/`: the guest benchmark, with VMM thread samples per guest run in `closed/` and `paced/`.
- `workloads/` and `workloads.log`: the issue 47 workloads at full size (4,000 callers; 10 million records of history; a 60 million record soak), all with zero correctness errors.
- `m1-capture/`: the issue 49 capture on this host, with `ROLL writer_ns` lines in each run log.
