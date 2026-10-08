---
id: 01K4FWRQ76H9C8ZK4HG0PSK8FX
created: 2025-09-06T13:11-03:00
---

# export-worker: uploads, finished state, revised exports

Rough notes on how export-worker handles uploads, when an export counts as finished, and what happens when an export is revised. Written from memory of reading the code, not re-verified line by line, so treat gaps as gaps.

## What export-worker is for

export-worker is the Elixir process group that turns a finished poll or Q&A session into a file an event producer can download. Producers ask for results after an event: poll tallies, the question list with moderation outcomes, and sometimes the raw vote log. The request comes in from the Next.js dashboard, goes through Phoenix, and lands as a job row in CockroachDB. The worker picks the row up, builds the file, uploads it, and marks the row.

## Job lifecycle

A job moves through a small set of states: queued, running, uploading, finished, failed. The names in the code may differ slightly from these. The important thing is that "finished" is only written after the upload step has confirmed. Earlier versions of the worker marked finished as soon as the file was built locally, and producers got links that did not resolve yet. That is no longer the case, but it is the reason the ordering matters.

## Upload step

The worker streams the built file to object storage in parts. Each part has a size set in config, and the number of parts in flight at once is also configured. If a part fails it is retried a few times with a backoff before the whole upload is abandoned. The upload target is picked per tenant, so check the tenant config before assuming where a file went.

Things to remember:

- The upload is not atomic from the outside. A partial object can exist in storage while the job is still in the uploading state.
- The worker cleans up its own partial objects on failure, but not if the node dies mid-upload. A sweep job is meant to handle that.
- The link handed to the dashboard is only created after the finished write.

## What "finished" means

Finished means: the file is fully in storage, its size and checksum were compared with what the worker computed locally, and the job row was updated in one transaction together with the download metadata. If any of those is missing, the job is not finished, whatever the logs say. The dashboard polls the row (and also gets a WebSocket push) and only shows the download button on finished.

## Revised exports

Producers can revise an event after the fact, for example when moderators reverse a decision on a question or a poll is corrected. When that happens the old export is not edited. A new job is created that points at the earlier one as its predecessor, and the new file gets uploaded under its own key. The old file stays available until the retention window passes.

The row for the older export is marked as superseded, not deleted. The dashboard shows the newest one by default and offers the older ones in a history list. I am not sure whether the superseded mark is set at queue time or at finished time of the new job; I believe finished time, which is the safer choice, since a failed revision should not hide a good export.

## Retries and idempotency

Jobs carry an idempotency key derived from the event, the export type and a revision counter. A duplicate request with the same key returns the existing job instead of making a new one. Bumping the revision counter is what makes a revised export a different job. Retried uploads reuse the same storage key for the same job, so a retry overwrites its own partial object rather than leaving strays.

## Failure handling

When a job fails, the row keeps the last error text and the stage it failed in. Producers see a generic message on the dashboard; the detail is only for operators. Failures at the build stage are usually data problems, such as an event with a huge vote log hitting the configured memory or time limit. Failures at the upload stage are usually storage credentials or network. The worker does not retry build failures automatically.

## Observability

Telemetry events are emitted at each state change, and there is a counter for jobs stuck in uploading longer than the configured threshold. That alert is the first thing to look at when producers say a download never appeared. Logs include the job id and tenant, which is enough to find the row in CockroachDB.

## Open questions

- Whether the sweep of orphaned partial objects runs often enough for events with very large exports.
- Whether superseded exports should count against the tenant storage quota during the retention window.
- Whether the WebSocket push on finished should also fire for revisions, or only the poll path should surface them.
- The exact point at which the superseded flag is written, as noted above; confirm in the code before relying on it.

## Where to look

The worker lives in the Elixir umbrella next to the other background workers; the upload module and the job state module are the two files that matter. Config for part size, concurrency, retry count and the stuck threshold sits in the runtime config for the worker. The dashboard side is a small Next.js page that reads the job row and renders the download list.
