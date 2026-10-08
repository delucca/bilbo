---
id: 01K2V93YV4A62BJQA397RNSDRN
created: 2025-08-17T02:47-03:00
sources:
  - "code: lib/townhall/exports/export_job.ex"
---

# export-worker design

export-worker is an `Oban` job on the exports queue with `concurrency 3` that streams CSV files out of CockroachDB. This note records how it is shaped and why. Nothing here is measured; it is the intended design as I understand it.

## Purpose

Event producers and community managers need poll results and Q&A transcripts after or during a large virtual event. export-worker produces those as CSV so they can open them in a spreadsheet. It runs outside the request path so Phoenix and the WebSocket connections are not slowed by big reads.

## Job runner

export-worker is an `Oban` job. Oban keeps jobs in the database, so a queued export survives a deploy or a node restart. The job is enqueued from the Phoenix side when a user asks for an export, usually from the Next.js dashboard.

## Queue and concurrency

Jobs go on the exports queue. The queue is set to `concurrency 3`, so at most three exports run at once per node's queue config. Exports are kept on their own queue so they cannot starve live moderation or poll work.

## Why a low limit

Each export holds a long read against CockroachDB. Many at once would compete with live traffic for connections and CPU. A small limit keeps the cost predictable during a busy event.

## Streaming

The worker reads rows in a stream and writes CSV as it goes. It does not load a whole result set into memory. This matters for large events with many questions and votes.

## Data source

All data comes from CockroachDB. The worker reads only; it never changes poll or Q&A records.

## Output format

Plain CSV with a header row. Text fields are quoted and escaped so questions with commas or line breaks stay in one cell.

## Example shape

```elixir
# queue config
queues: [exports: 3]
```

This sets the exports queue to run three jobs at once, which is the `concurrency 3` above.

## Failure and retry

If a job fails, Oban retries it. A retry should start the file again from scratch rather than append to a partial one. Keep this in mind if output is ever written to shared storage.

## Moderation

Removed or hidden content should follow the moderation state at the time of export. Check this before changing the query; it is easy to leak removed items.

## Open questions

- Where finished files live and how long they are kept.
- How users learn an export is ready.

## Gotchas

- Raising concurrency without looking at database load can hurt live events.
- Do not run exports on the default queue.

## Related

Other components that touch this: the Phoenix channels that serve live polls and the Next.js dashboard that requests exports.
