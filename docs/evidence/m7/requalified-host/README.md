# M7 requalification on one host

The raw output of `scripts/m7-session.sh` for the [qualification record](../qualification.md), run on 2026-11-17 and 2026-11-18 ([issue 112](https://github.com/ironflowx/segment-log/issues/112), [issue 133](https://github.com/ironflowx/segment-log/issues/133)). Every row of the record comes from this one host, as [ADR 0059](../../../decisions/0059-m7-qualifies-firecracker-on-a-dedicated-nvme-host.md)'s amendment requires. Each step wrote `<step>.log`; [results.tsv](results.tsv) has each step's exit status and its end time in UTC.

## Host

Hetzner server-auction machine #3115170, FSN1, used on 2026-11-17 and 2026-11-18 and cancelled after the session ([host.txt](host.txt)).

- AMD EPYC 7401P (24 cores, 48 threads), 128 GB. Debian 12, kernel `6.1.0-54-amd64`, installed by `m7-session.sh bootstrap`.
- Two Samsung PM983 `MZQLB960HAJR-00007` U.2 drives, firmware `EDA5502Q`, about 62,650 power-on hours, 9 % used ([smart-rescue.txt](smart-rescue.txt)). `vwc 0`: no volatile write cache. `awupf 7`, `nawupf 7`. The same model and firmware as the first qualified host.
- The Robot web service offers `power`, `power_long`, `hw` and `man` resets for this server.
- The volumes are raw images on ext4 on the drive that does not hold `/`, `rw,noatime`, no RAID.
- Firecracker v1.18.0 and the guest artifacts from `scripts/m7/setup.sh`, at `386695f`: format version 3, protocol version 2.
- The observer was a Hetzner Cloud server in Ashburn, separate from the host and the controller, deleted after the session.
- [fingerprint](fingerprint) is the pinned copy that `bootstrap` wrote and `collect` saved; it is committed as `scripts/m7/fingerprint`.

## The power cuts removed drive power

[smart-before.txt](smart-before.txt) was taken after `bootstrap`, before any step; [host.txt](host.txt) after the session. Both drives went from `power_cycles` 16 to 47 and from `unsafe_shutdowns` 2 to 33: 31 each, the 30 cuts of `hc-host-power-loss` and its 1 control cycle. The host kernel crashes, resets and recovery cuts in the same session added none.

## The first `v2-clean` run

`v2-clean` passed its first 9 cycles (0 missing of 46,624 records) and then stopped at cycle 10 with `check exit 255`: the host had crashed and booted, but the ssh session that runs the check timed out ([check-10.log](v2-clean-20261117224447/check-10.log)). The harness counts a check that did not run as a broken harness, never a pass. After `collect`, `v2-clean` ran again on the same host as [v2-clean-rerun.log](v2-clean-rerun.log); the record's V2 row uses the rerun.

## Files

- `rehearse.log`: V1 flush configuration, V4, V5, V6 and the conformance cases over vsock.
- `pc-*.log`: guest-side power cuts; `hc-*.log`: host kernel crash, host reset and host power loss. Each has a `-control` pair.
- `v2-*.log` and the `v2-*` directories: recovery cuts, one stream and one check log per cycle.
- `latency-*.log`, `metadata*.log`, `enospc-*.log`, `capacity.log`, `drift.log`.
- `guest-bench/` and `guest-bench-per-caller/`: the guest benchmark at 16 connections and at one connection per caller, with VMM thread samples.
- `workloads/` and `workloads.log`: the issue 47 workloads at full size, all with zero correctness errors.
- `m1-capture/`: the issue 49 capture, with `ROLL writer_ns` lines in each run log.
- [session.sh](session.sh) and [session.log](session.log): the driver and its output.
