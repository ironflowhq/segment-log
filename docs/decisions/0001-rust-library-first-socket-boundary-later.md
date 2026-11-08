---
id: "0001"
title: "Rust library first, socket boundary later"
status: accepted
date: 2026-11-07
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [packaging]
touches: [segment-log-core/**, segment-log-server/**]
enforced_by: none
---

# 0001. Rust library first, socket boundary later

## Context

Source: from the reviewed design (decision 1 and the language boundary).

The segment log must be small, start fast, and run as the whole process inside a throwaway microVM guest. Its hot path is one writer thread that packs submits into groups and calls `fsync`. Callers in other languages must reach it too. The first consumer is written in Go, and it builds without cgo, so it cannot link a C library in process.

## Options & trade-offs

### Option A - Rust library, with a socket server as a separate later crate

- **For:** no garbage collector and no runtime on the write path. The guest can be one small binary. A separate process can be sealed and killed without the orchestrator. Works for callers in any language.
- **Against:** a socket round trip per call for callers outside the process. Two crates to maintain.

### Option B - Rust library with an in-process C ABI

- **For:** no socket round trip.
- **Against:** the first consumer cannot use it without cgo. A crash in the log takes down the caller. The log cannot be sealed and killed on its own.

### Option C - write the log in Go, next to its first consumer

- **For:** one language for the consumer.
- **Against:** a garbage collector and a larger runtime in the guest. No gain on the `fsync` cost, which is the real limit.

## Decision

We will build a Rust library first and add the socket server later, as a separate crate (option A). We accept a socket round trip per call for callers outside the process. `segment-log-core` is a Rust library that uses only `std` in M1. The write path has no async runtime, no SQL and no protobuf. `segment-log-server` exposes it over a Unix socket or vsock in M6. Until then, the server crate stays an empty placeholder, and no work on it starts before the M1 crash tests pass.

## Consequences

- All of M1–M5 is library work and tests.
- The socket protocol is a later decision, with its own ADR.
- Callers in Rust can link the core crate directly.

## Enforcement

None. A reviewer checks that `segment-log-server` stays a placeholder until M6.

## Revisit if

- A consumer must call in process, and a measurement shows that the socket round trip misses a PRD target.
- The first consumer starts to allow cgo.
