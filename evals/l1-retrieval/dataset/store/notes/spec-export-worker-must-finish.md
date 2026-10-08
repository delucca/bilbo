---
id: 01KESEBX5S29XCE9GK7DWWD0K9
created: 2026-01-12T12:49-03:00
---

# export-worker spec: size and time target

The main requirement for export-worker is a throughput target. export-worker must finish exports of up to 500000 rows within 10 minutes. That is the whole contract: a request that stays at or under 500000 rows has to be fully written and available to the requester within 10 minutes of the job starting to run. A bigger export is not covered by the guarantee. It may still run, but nobody should promise a time for it, and the product should not show one.

This note is written in a hurry from what we settled. It is a spec, not a design. It says what export-worker has to do and how we will know it does it. How it does it is left open on purpose, apart from the constraints below that follow from the stack.

## What the target covers

The figure of 500000 rows is the ceiling for the guarantee. The 10 minutes is wall-clock time for one export job, measured from the moment export-worker picks the job up to the moment the finished file can be downloaded. Time spent waiting in the queue is a separate thing. If the queue is backed up, the user waits longer, and that is a capacity problem to fix elsewhere. It does not count against the 10 minutes. We should still record queue wait next to run time, so that a slow export is never blamed on the wrong part.

A row is one record in the result of the export query. For TownHall Pulse that means things like a poll response, a Q&A question, a moderation action, or an attendee entry, depending on what the producer asked for. A single export has one row type. An export that joins several kinds of data counts the rows of the final result, not the rows of the source tables. This matters because a Q&A export with votes and moderation history can have many more source rows than output rows, and the target is about output.

The target applies to a normal load. Normal load means other exports are running and a live event is in progress, because that is when producers and community managers ask for exports. A test that only passes on an idle system does not show the target is met. The worst case that matters is an export requested near the end of a big event, while the same database is serving live polls and moderation traffic.

What the target does not cover:

- Exports above 500000 rows. These are allowed to be slower. They may also be refused or split, see the open questions.
- Time to deliver the file to the user's browser. That depends on their connection.
- Time for the Next.js front end to show the finished state. That should be fast, but it is not part of this spec.

## Constraints from the stack

The data lives in CockroachDB. The service is written in Elixir on Phoenix. Live updates reach the browser over WebSockets. The front end is Next.js. Each of these shapes how export-worker should behave.

CockroachDB is distributed, and a big read touches many ranges on many nodes. A single huge query that pulls everything into memory is the wrong shape. export-worker should read in bounded batches, ordered by a stable key, and resume from the last key it saw. That keeps memory flat and makes a retry cheap. It also avoids long-lived transactions, which in CockroachDB tend to run into contention and retries when live writes are going on. An export does not need a perfectly consistent snapshot of a changing event unless the producer asked for a final report. For a running event, a read at a fixed point in time is enough, and it is consistent without blocking the writers. We want the point in time chosen once at the start of the job and reused for all batches, so the export does not contain a mix of old and new data. Reading at a slightly past time is also kinder to the cluster than reading the latest data.

On the Elixir side, export-worker should stream. Rows go from the database to the output file in batches and are not collected into a list first. The BEAM handles many processes well, so parallel reads over key ranges are possible, but the order of the output file must stay stable. If we split by range and join the pieces, the join has to keep the order the user expects. We should only add parallelism if a plain streaming version misses the target. Start simple, measure, then add it.

The export must not slow the live path. Polls and Q&A moderation are real-time, and a producer would rather have a late export than a laggy poll. So export-worker runs as a separate worker, with its own limits on concurrency and its own database connection pool. It should not share a pool with the request handlers that serve WebSocket traffic. If the database is under pressure, export-worker backs off. It does not push harder. Missing the 10 minutes on a busy day is better than hurting a live event, and we should say so in how we describe the feature. Even so, the target is meant to hold under normal load, so backing off should be rare and should be visible in metrics when it happens.

Next.js and the Phoenix channel layer only deal with job status. The browser asks for an export, gets a job reference back, and then receives progress and completion over the socket, or by polling if the socket is down. The file itself is not sent through the socket. It is stored and handed out by a download link with a limited lifetime. export-worker does not need to know anything about the browser beyond the status events it publishes.

## How we check it

The target has to be checked with a test that runs at the full size. A test with a small data set and a straight-line guess at the rest is not enough, because the cost of a big export is not linear. Batches, file writes, and cluster behavior all change at scale.

The plan is a load test that does the following:

- Seed a table so that one export returns exactly 500000 rows, with realistic row width. Short, uniform rows make the test too easy. Use rows that look like real Q&A text with a long tail of lengths.
- Run the export while a synthetic live event is hitting the same cluster with poll votes, questions, and moderation actions at a rate close to a big event.
- Record queue wait, run time, rows written, batch retries, and peak memory of the worker process.
- Pass if run time is under 10 minutes on every run in a series, not just on average. A slow tail is the thing producers will notice.

We should also run the test with several exports at once, since after an event ends many producers ask for their data around the same time. The spec does not set a number of simultaneous exports. It says that each export of up to 500000 rows must meet the target when the system is at its normal concurrency limit. If we cannot hold that, the concurrency limit gets lowered. We do not relax the target for the exports that do run.

Metrics to keep in production, so that we can notice drift: run time per job, rows per job, queue wait per job, and the count of jobs that finished close to the limit. An alert should fire when jobs at or below 500000 rows start to take a large share of the allowed time, well before they miss it. A slow creep is much more likely than a sudden break, since data and event sizes grow over time.

## Failure behavior and open questions

If an export fails part way, export-worker should resume from the last committed batch and not start over. A restart from the beginning could use up the whole budget by itself. The time budget of 10 minutes is for the job as a whole, retries included. A job that has used up its time without finishing is marked as failed with a clear reason, and the user is told so. We do not leave it running forever and we do not hand over a half file as if it were complete. A partial file can be offered only if it is clearly labeled as partial, and that is not decided yet.

Duplicate requests should not cost twice. If a user asks for the same export again while one is running or has just finished, they should get the existing job. This keeps the load down when someone clicks the button more than once, which will happen.

Open questions, not decided:

- What to do above 500000 rows. Options are to refuse with a message, to run without a time promise, or to split into several files. The choice is for product, but engineering prefers splitting, because it keeps each part inside the guarantee.
- Whether the file format changes the cost enough to need its own target. The target currently assumes the common tabular format. A format with heavier encoding might need more time per row.
- Whether the point-in-time read is acceptable for every export type, or whether the final report for a finished event should wait until the event is closed and use the final data.
- Whether to show an estimated time to the user. If we do, it should be based on measured speed of recent jobs and not on the 10 minutes figure, which is a limit and not an average.

Until those are settled, the one fixed fact is the target itself: export-worker finishes exports of up to 500000 rows within 10 minutes, under normal load, and yields to live traffic when the two conflict.
