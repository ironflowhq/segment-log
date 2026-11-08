---
id: "0005"
title: "Flush calls and supported platforms"
status: proposed
date: 2026-11-07
deciders: []
supersedes: null
superseded_by: null
tags: [durability, platform]
touches: []
enforced_by: none
---

# 0005. Flush calls and supported platforms

## Context

Source: new in this repository. Not reviewed. The source design left open "`fsync`, `fdatasync`, or `F_FULLFSYNC` on macOS".

The target is a Linux guest on a microVM with a virtio block device. Developers work on macOS too. On macOS, `fsync` does not flush the drive's write cache. `F_FULLFSYNC` does.

In the Rust standard library that the pinned toolchain ships, `File::sync_data` calls `fdatasync` on Linux and `fcntl(F_FULLFSYNC)` on Apple targets. `File::sync_all` calls `fsync` on Linux and `fcntl(F_FULLFSYNC)` on Apple targets. (Read in `library/std/src/sys/fs/unix.rs`, Rust 1.99.1.) Both calls go through `cvt_r`, which repeats the system call while it fails with `EINTR`. An `EINTR` means that the call did not complete, so this is not a retry after an error. On Linux, `fdatasync` flushes the data and the metadata needed to read it back, which includes the file size.

## Options & trade-offs

### Option A - `std` calls only (`sync_data` for segment data, `sync_all` for directories and metadata changes)

- **For:** no dependency. Correct on Linux and macOS.
- **Against:** no control over the exact call. That is acceptable while `std` maps it as above.

### Option B - direct `libc` calls

- **For:** full control, for example `sync_file_range` or `O_DIRECT`.
- **Against:** a dependency and platform code, with no measured gain.

## Decision

We will use only the flush calls in `std` (option A). We accept no control over the exact system call, while `std` maps it as Context describes.

- After writing a group: `sync_data` on the segment.
- After truncate in recovery: `sync_all` on the segment.
- After creating, renaming or removing a file: `sync_all` on a handle to the directory.
- Linux (ext4 and xfs) is the supported platform for the PRD targets. macOS works for development and its tests run in CI. Other platforms are not supported.
- If the guest's block device does not honor the flush, the contract does not hold. The library cannot detect this. Each deployment must check it (PRD open item 3).

M1 must still verify:

- with `strace` on Linux, that each new group commit makes exactly one `fdatasync` call, distinguishing initialization and recovery syncs;
- that a directory sync through `File::open(dir)?.sync_all()` works on both platforms during creation and recovery.

M3 must verify with `strace` on Linux that a roll makes the required directory `fsync`. Roll does not exist in M1.

## Consequences

- [ADR 0032](0032-qualify-deployment-storage-before-m6.md) requires actual-stack qualification before the M6 guest benchmark and production durability claims. M1 syscall and directory-sync evidence does not qualify the guest's underlying storage.
- [ADR 0029](0029-m1-completion-requires-correctness-and-measurement-evidence.md) makes M1 platform evidence part of the completion gate and moves the roll-specific trace to M3.
- Benchmarks on macOS measure `F_FULLFSYNC`, which is much slower than Linux `fdatasync`. They are not used for the targets.

## Enforcement

None yet. The M1 `strace` check in Decision will verify the calls on Linux once. Nothing checks that the mapping in `std` still holds after a toolchain update.

## Revisit if

- A measurement shows that `O_DIRECT`, `io_uring` or `sync_file_range` meets a target that `std` misses.
- A future `std` changes the mapping above.
- A platform other than Linux and macOS is needed.
