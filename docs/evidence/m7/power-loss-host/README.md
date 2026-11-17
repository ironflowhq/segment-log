# M7 host power loss, and the guest benchmark before and after ADR 0063

The [qualified host](../qualified-host/README.md)'s Robot web service offers no power button reset, so host power loss could not run there. This host, ordered on 2026-11-17 and cancelled the same day, offers `power` and `power_long` (`GET /reset` lists `power`, `power_long`, `hw`, `man`). It ran the three host events with their controls, then the guest benchmark before and after [ADR 0063](../../../decisions/0063-the-guest-benchmark-shares-pipelined-connections.md), and the guest-side power cuts on the changed server. The [qualification record](../qualification.md) cites these files.

## Host

Hetzner server-auction machine #3115035, FSN1 ([host.txt](host.txt)).

- AMD EPYC 7401P (24 cores, 48 threads), 128 GB. Debian 12, kernel `6.1.0-54-amd64`, installed by `m7-session.sh bootstrap`.
- Two Samsung PM963 `MZQLW960HMJP-00003` U.2 drives, firmware `CXV8601Q`, about 66,870 power-on hours. `vwc 0`: no volatile write cache. `awupf 0`, `nawupf 7`. The qualified host has PM983 drives: same vendor and class, different model.
- The volumes are raw images on ext4 on the drive that does not hold `/`, `rw,noatime`. NVMe names swapped across the install reboot here, so `bootstrap` now picks that drive by mount instead of by name.
- Firecracker v1.18.0 and the guest artifacts from `scripts/m7/setup.sh`, at `0a41cff` for the host events and the "before" benchmark, at `2b378c7` for the rest.
- The observer was a Hetzner Cloud server in Ashburn, deleted after the session.

## The power cuts removed drive power

[smart-before.txt](smart-before.txt) and [smart-after.txt](smart-after.txt) bracket the host events. Both drives went from `power_cycles` 54 to 85 and from `unsafe_shutdowns` 28 to 59. That is 31 each: the 30 real cuts and the 1 control cycle. The 31 host kernel crashes and 31 resets in the same session added none. So each `power_long` press cut power to the drives with no shutdown notice, which is the evidence V7 asks for.

## Files

- [session.log](session.log), [results.tsv](results.tsv) and [run.sh](run.sh): the host events (`hc-host-power-loss`, `hc-host-kernel-crash`, `hc-host-reset`, each with `-control`).
- [followup.log](followup.log) and [followup.sh](followup.sh): the benchmark before and after ADR 0063, and the rehearsal and guest-side power cuts on the changed server.
- `before/`: the guest benchmark at `0a41cff`, 1,000 connections.
- `after/`: at `2b378c7`: `rehearse.log`, `pc-*.log`, `guest-bench/` (16 connections) and `guest-bench-per-caller/` (1,000 connections).
