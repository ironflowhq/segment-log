---
id: "0000"
title: <What we are saying no to, in one line>
status: proposed        # proposed | accepted | superseded | deprecated
date: YYYY-MM-DD
deciders: [<who actually made the call>]
supersedes: null        # id of the record this replaces, e.g. "0003"
superseded_by: null     # id of the record that replaced this one
tags: []                # e.g. [database, auth, billing, marketing, jobs]
touches: []            # paths or globs this decision governs, e.g. [apps/nextjs/drizzle/**]
enforced_by: none          # none | lint | test | ci. Name the file in Enforcement below
---

# 0000. <What we are saying no to, in one line>

## Context

What situation forces a decision now? State the constraints, the pressure, and
anything already fixed that we cannot change. Facts only, no advocacy. A reader
who joins in a year should understand the problem without reading the code.

## Options & trade-offs

### Option A - <name>

- **For:** …
- **Against:** …

### Option B - <name>

- **For:** …
- **Against:** …

### Option C - <name>

- **For:** …
- **Against:** …

List the options that were genuinely on the table, including the one we rejected
most reluctantly. An option with no honest downside means the list is incomplete.

## Decision

We will <do X>.

One paragraph on why this option beat the others, tied to the constraints in
Context. Name the trade-off we accepted on purpose.

## Consequences

What becomes true once this lands. Include the costs, not just the wins:

- What gets easier.
- What gets harder or slower.
- What now has to be maintained, migrated, or watched.

## Enforcement

What stops this decision from quietly reversing: a lint rule, a test, a CI
job. Name the file. If nothing does, write `None` and say what would. An ADR
with no enforcement is a wish, and a reviewer has to remember it.

## Revisit if

The concrete signals that would reopen this decision:

- <a threshold crossed, e.g. "write volume passes N/day">
- <an assumption breaks, e.g. "the vendor drops the free tier">
- <a date, e.g. "by YYYY-MM-DD if X has not happened">
