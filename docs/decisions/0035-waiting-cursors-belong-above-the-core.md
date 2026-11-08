---
id: "0035"
title: "Waiting cursors belong above the core"
status: accepted
date: 2026-11-08
deciders: [Altug Sahin]
supersedes: null
superseded_by: null
tags: [api, read-path, architecture, milestones]
touches: [docs/prd.md, docs/architecture/api.md, docs/architecture/overview.md, docs/architecture/testing.md, docs/glossary.md]
enforced_by: none
---

# 0035. Waiting cursors belong above the core

## Context

PRD open item 6 leaves ownership of a cursor that waits for watermark movement undecided. The architecture already places subscriber state and event-waiting policy above the log. M1 exposes a watermark and bounded reads, and `Pending::wait` waits for one submitted request's result.

The owner approved keeping waiting cursors above the core, with no native watermark-wait API in M1, during the readiness discussion on 8 November 2026.

## Options & trade-offs

### Option A - waiting cursors in the consuming layer

- **For:** cursor positions, cancellation, timeouts and scheduling follow the consumer's needs. M1 avoids another blocking API and its lifecycle rules.
- **Against:** an initial wrapper may poll. Polling trades notification latency against polling work, without an efficient native watermark notification.

### Option B - add a native watermark-wait API in M1

- **For:** consumers can wait for new durable records without polling the core.
- **Against:** needs additional timeout, shutdown and poison behavior before a concrete consumer has established those requirements.

## Decision

Waiting cursors belong in the layer consuming the log. That layer owns cursor positions, polling or scheduling, cancellation and timeouts. M1 keeps the existing `watermark()` and bounded `read()` calls without adding a native watermark-wait or subscription API.

`Pending::wait` is unchanged. It waits for a particular submit's result, including a matched retry's durability result. It is not a cursor waiting for arbitrary future records.

A consumer may initially use bounded polling through the existing read API. This decision does not add a wrapper, polling interval or waiting endpoint to M1 or M6. A future need for efficient wakeups can motivate a separate decision for a minimal core notification primitive, while keeping cursor and subscription policy outside the core.

We accept leaving notification efficiency to a later demonstrated need rather than defining a new blocking contract during M1.

## Consequences

- PRD open item 6 is resolved for ownership and M1 scope.
- The core gains no subscriber registry, cursor state, timeout policy or runtime dependency from this decision.
- M1's reader durability rules remain unchanged. A wrapper can only expose records that the published watermark and `read` permit.
- A later notification primitive requires a concrete consumer need and a separate API decision. It is not promised by this milestone.

## Enforcement

None yet. The M1 reader oracle in [testing.md](../architecture/testing.md#the-oracle) will verify durable visibility through the existing API. Review must keep waiting-cursor state and policy outside the core. Any future consumer wrapper needs its own behavior tests for waiting, cancellation and timeouts.

## Revisit if

- A concrete consumer and measurements show that polling cannot meet its requirements. Evaluate a minimal notification primitive without moving subscription policy into the core.
