---
id: 01K5APEG3WFV154HX0FX10FC2F
created: 2025-09-16T23:00-03:00
---

# export-worker design notes

The export-worker is the background piece that turns a finished poll or Q&A session into a file a producer can download. It runs as Oban jobs inside the Phoenix app and reads from CockroachDB. These are loose notes, not a full spec.

## Why it exists

Producers and community managers ask for results after an event: votes per option, the question list, moderation actions. Building that inside a request would time out on large events, so the export-worker does it off to the side and tells the Next.js front end when it is ready.

## Job shape

One Oban job per export request. The args carry the event reference, the export type and who asked. The queue for exports is separate from the live moderation queue, so a big export cannot starve moderation work. Concurrency on that queue is kept low on purpose; the configured limit is what we usually run with.

## Reading the data

Reads should be paged, not one huge query. CockroachDB is fine with this if we keep to a stable ordering key and avoid long-held transactions. For consistency we read at a single point in time so the numbers do not shift while the file is being written. Worth checking whether follower reads are good enough here, since the export does not need the very latest row.

## Output and storage

The file is streamed out as it is built, so memory stays flat even for the biggest events. It goes to object storage and the job records where it landed. Links given to the user expire after the usual short window.

## Retries and failure

Oban retries on failure up to the configured attempt count, with backoff. The job has to be safe to run twice: the same request should overwrite the same output location rather than create a second file. If the data read fails midway, we start over rather than resume. Not sure yet if resume is worth the complexity.

## Notifying the client

When the job finishes it broadcasts over Phoenix PubSub, and the channel pushes a message down the WebSocket to the requester. If they are not connected, the front end just checks status when the page loads. Do not rely on the push alone.

## Permissions

Only people with the right role on the event can request or fetch an export. The check happens at request time and again when the download link is made, since roles can change between the two.

## Open questions

- Should very large exports be split into several files?
- Do we want a cap on how many exports one user can have running at once?
- How long should finished files be kept before cleanup?
- Do moderation logs belong in the same export or a separate type?
