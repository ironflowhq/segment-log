# M3 completion record

M3 (segment roll) passed on 2026-11-10. This record links the evidence for each part of the [M3 gate](../../prd.md#milestones).

| Gate part | Evidence |
| --- | --- |
| No submit crosses a roll in the crash tests. | The M3-ROLL scenarios in [testing.md](../../architecture/testing.md#milestone-acceptance-coverage). [CI run 42925746286](https://github.com/ironflowhq/segment-log/actions/runs/42925746286) is green on Ubuntu and macOS at `bb38e4b`. CI also reruns the whole suite with `roll_bytes` at 65 and 256 bytes. |
| A crash at each step of a roll recovers. | The roll crash and fault scenarios in the same run. Each crash point and each injected I/O error at a roll step recovers every acknowledged record. |
| A Linux syscall trace verifies directory sync during roll. | [M3 evidence run 42925774979](https://github.com/ironflowhq/segment-log/actions/runs/42925774979) at `bb38e4b` on ubuntu-24.04. [`syscalls.trace`](syscalls.trace) is the strace output. [`syscalls-verified.txt`](syscalls-verified.txt) is the output of `scripts/m3-verify-roll-syscalls.py`. In the roll phase, it shows the new segment created, its header written, `fdatasync` on the new segment, and `fsync` on the log directory, all before the first group frame is written to the new segment. |

[`environment.txt`](environment.txt) records the runner, the kernel and the toolchain. [`binary.sha256`](binary.sha256) identifies the traced test binary.

The changes that come after `bb38e4b` change only documents.
