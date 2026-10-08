---
id: 01KXJWSSB64FMPWEYVBSHKQVZY
created: 2026-07-15T09:42-03:00
---

# export-worker throughput target

This note replaces the earlier note "export worker must finish". The new value: export-worker must finish exports of up to 2000000 rows within 15 minutes. The old target of 500000 rows within 10 minutes is gone and should not be used for anything.

## Requirement

export-worker has to complete any export of up to 2000000 rows within 15 minutes. That is the whole contract. Anything larger than that is outside the target and may take longer without counting as a miss.

## What changed

The earlier target was 500000 rows within 10 minutes. The row ceiling went up and the time budget went up too, but by less than the row count did. So the required throughput is higher than before. Anyone sizing the worker from the old note will undersize it.

## Why it matters

Producers running large virtual events want the full poll and Q&A record soon after the event ends. Community managers use the same exports for moderation review. Long waits for big events were the complaint that drove the change.

## Scope of the target

The target covers the export-worker run from start to finished file. It does not cover time a job sits in a queue before the worker picks it up. If queue wait becomes a problem, that needs its own number.

## Data source

Rows come from CockroachDB. Reads should be streamed or paged rather than loaded whole, because a result set of this size will not fit comfortably in memory on a small node.

## Memory

Keep memory flat as the row count grows. The worker should hold a bounded batch at a time and write out as it goes. If memory grows with row count, the target will be missed through swapping or restarts long before the time limit.

## Elixir side

export-worker runs in the Elixir and Phoenix backend. Use back-pressure between the reader and the writer so the reader cannot run far ahead. Concurrency should be tuned by measurement, not guessed.

## Database load

Exports must not hurt live events. Heavy reads against CockroachDB during a live poll can slow voting and moderation. Prefer reading in a way that avoids contention with live writes, and consider running big exports off the hot path.

## Progress reporting

The Next.js front end shows export status. The worker should report progress often enough that the page never looks stuck during a long run. Real-time channels over WebSockets can carry that progress.

## Failure handling

A failed export should be retryable without starting the whole thing from scratch if that is practical. At minimum, a retry must not leave a partial file that looks complete.

## Timeouts

Any timeout inside export-worker has to be consistent with the 15 minutes budget. A shorter internal timeout will kill valid jobs. A much longer one hides stuck jobs.

## How to verify

Run a test export at the full 2000000 rows and time it end to end. Also run a smaller export to check scaling is roughly linear. Record the results next to this note when they exist.

## Test data

Use generated data that looks like real events: many participants, many poll answers, long question text. Tiny uniform rows make the numbers look better than they are.

## Open points

Output format limits, compression, and file size caps are not settled here. Measured results may force a change to the target.

## Related notes

The earlier note about the old target is superseded by this one. If it still exists in the store, treat this note as the source of truth.

## Ownership

Whoever changes export-worker should re-check this target before merging. Update this note instead of making a new one when the target moves again.

## Status

The target is agreed. No measured run against 2000000 rows is recorded yet.
