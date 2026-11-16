# V3 and V9 on the ADR 0059 stack, rehearsed in Lima

`scripts/m7-metadata-error.sh` (V3) and `scripts/m7-enospc.sh` (V9) run in the Mac's Lima VM with nested KVM, on 2026-11-16. They use the M7 guest: Firecracker v1.18.0, guest kernel 6.1.155, ext4 mounted `rw,relatime,errors=remount-ro`. Lima is a rehearsal host, so these results fill no qualification row. They show what the dedicated host will find, and V3's result does not depend on the host.

## V3 `VOL-METADATA-ERROR`: fails under every `errors=` mode

The guest kernel has no device-mapper, so the failing target is a dm table on the host, under the guest's virtio drive. The guest sees the failures as I/O errors (`I/O error, dev vdb … op 0x1:(WRITE)`, `Aborting journal on device vdb-8`), and ext4 remounts read-only.

| Mount | Step 2 directory fsync | Step 4 fsync, held handle | Step 4 fsync, new handle | Step 4 file write | Result |
| --- | --- | --- | --- | --- | --- |
| `errors=remount-ro` (qualified) | EIO | **ok** | **ok** | EROFS | Fail |
| `errors=continue` (control) | EIO | ok | ok | EROFS | Fail; the journal abort makes it read-only too, so no distinct control |
| `errors=panic` (separate procedure) | EIO | ok | ok | panic | Fail: the panic comes at the step 4 file write, after both fsyncs returned |
| `errors=remount-ro`, guest kernel 5.10.245 | EIO | ok | ok | EROFS | Fail |

After the journal aborts, the filesystem is read-only, and an fsync that has nothing to write returns success. V3 requires that every call after the failure fails until remount, so V3 fails on this filesystem and kernel. Every write fails with EROFS, so a writer cannot append after the failure. The log poisons on the first failed flush and never retries one ([ADR 0004](../../../decisions/0004-io-errors-poison-the-writer.md)). Whether that is enough is a requirement decision, not a test result: see issue 112.

## V9 `VOL-ENOSPC`: passes for both layers

An unpaced load with 4 KiB payloads fills the layer, the writer poisons (`PoisonedBeforeWrite`), the script restores capacity and reopens the volume.

| Layer that fills | Acknowledged and reported | Present after reopen | Open |
| --- | ---: | ---: | --- |
| Guest filesystem (96 MiB image, then grown to 512 MiB) | 21,078 | 21,078 | no `Corrupt` |
| Host filesystem under the sparse image (160 MiB, then grown online to 2 GiB) | 24,536 | 24,536 | no `Corrupt` |

The sizing record and the alert thresholds of `VOL-CAPACITY` need the deployment's workload profile and response time, which are not set yet.
