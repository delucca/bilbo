---
id: 01K0Y23ZVCZM1XYSQQ9902BJ05
created: 2025-07-24T08:11-03:00
---

# upload-gateway spec

This is the working spec for the upload-gateway, the service in ReelForge that accepts source video from publishers and hands it to the transcode pipeline. It is written in a hurry from what the team has settled so far, so treat gaps as open questions rather than as decisions. The one hard number in here is the size limit: the upload-gateway must reject any upload larger than 50 GiB with HTTP status 413. Everything else is described in general terms on purpose, because the exact values are tuned per deployment and live in configuration, not in this note.

The upload-gateway is written in Rust. It sits in front of AWS S3 for storage and starts AWS Step Functions executions that drive FFmpeg transcoding and HLS packaging. Media operations teams at independent publishers are the people who feel its behavior, so error messages and limits need to be clear to them.

## Purpose

The upload-gateway is the only door for source media. Publishers send a file, the gateway checks it is acceptable, stores it durably, and tells the pipeline that new work exists. It does not transcode anything and it does not package anything. Keeping it narrow means it can stay up when the heavy stages are slow or broken, and it means a failed transcode never loses the original upload.

The gateway should be boring. It validates, stores, records, and notifies. Anything clever, such as probing the media or choosing a ladder, belongs downstream. If a change makes the upload-gateway need to understand codecs deeply, that is a sign the change is in the wrong place.

## Size limit

The upload-gateway must reject any upload larger than 50 GiB with HTTP status 413. This is the central rule of this spec. A file of exactly 50 GiB is accepted; anything above it is refused. The limit applies to the total size of the source object, not to a single request or part, because uploads can arrive in many pieces.

The rejection must happen as early as possible. When the client declares a size up front and it is over the limit, the gateway answers with 413 before reading the body or creating any storage object. When the size is only known as bytes arrive, the gateway counts what it receives and stops the moment the running total passes 50 GiB, then answers 413 and cleans up whatever it wrote.

## Why the limit exists

Larger sources cause trouble further down the chain. Transcode workers have bounded scratch disk, FFmpeg runs on a single large input get very long, and a failure late in a huge job wastes a lot of compute. Publishers who send something above 50 GiB nearly always have a raw camera master that should be proxied first. The limit pushes them to send a reasonable mezzanine file instead.

It is also a protection against runaway or hostile clients. Without a cap, a single bad client could fill storage or hold connections open for a very long time. The cap is a cheap guard that does not need any per-account logic.

## What a 413 response looks like

The response status is 413. The body is a short structured error that says the upload was too large and states the allowed maximum as 50 GiB, so a media operator reading it knows what to do without opening a ticket. It should not leak internal bucket names, request identifiers meant for engineers only, or stack information.

The response should include enough to correlate with logs, using the same request correlation field every other response uses. The wording should be plain: the file is bigger than the limit, here is the limit, split or compress the source and try again. Do not blame the user and do not suggest contacting support as the first step.

## Where the check runs

There are two checks and both must exist. The first is on declared size, taken from the client's stated length when it gives one. The second is on actual bytes received, which is the one that cannot be fooled. A client that declares a small size and then sends more must still get 413 once the real total passes the limit.

For multipart style uploads, the gateway tracks the cumulative size across parts and also checks at completion time. The completion check is the last line of defense: if the sum of the parts is over the limit, the gateway refuses to finalize, answers 413, and aborts the multipart session so no orphaned parts remain in S3.

## Upload flow

A normal upload goes in this order. The client opens an upload, the gateway authenticates it and checks declared size, the bytes are written to S3 either streamed through the gateway or sent directly with a presigned grant, the gateway verifies the final object, records the upload, and then starts the pipeline. Only after the record is durable does the gateway tell the client it succeeded.

If any step fails, the client gets a clear error and the gateway cleans up what it created. The flow must be safe to retry from the client side, so each step either is idempotent or is guarded by an upload identity issued at the start.

## Direct to storage option

For big files it is better that bytes do not pass through the gateway at all. The gateway can hand the client a time limited grant to write straight to S3. In that mode the gateway cannot count bytes as they flow, so the size rule has to be enforced another way: the grant carries a size condition set to the limit, and the gateway re-checks the stored object size after the client says it is done.

If the stored object turns out to be over 50 GiB despite the condition, the gateway deletes it and answers 413. This should be rare, but the check must exist, because the condition on a grant is a convenience and not the source of truth.

## Authentication and ownership

Every upload belongs to an account. The gateway identifies the caller before it accepts any bytes and tags the stored object and the upload record with the owning account. An unauthenticated request is refused before the size check, so the size limit message is never shown to someone who is not allowed to upload at all.

Per-account quotas, if added later, are separate from the size limit. A quota failure must not reuse status 413, so clients can tell a single file that is too big from an account that has used up its space.

## Validation beyond size

Size is not the only thing the gateway checks. It confirms the declared content type is one the pipeline can handle, that the filename is sane and does not become part of a storage key without being cleaned, and that the object is not empty. It does not decode the media. A file that passes these checks but later fails FFmpeg is the transcode stage's problem to report.

Order of checks matters for user experience: authentication first, then size, then type. The cheapest and most certain refusals come first so the client learns the main problem quickly.

## Storage layout

Source objects land in a dedicated S3 bucket separate from outputs. Keys are built from the account and the upload identity, never from user supplied names alone, so two uploads with the same filename never collide. The original filename is kept as metadata for display.

Objects are written once and not modified. Lifecycle rules handle abandoned multipart sessions and unreferenced objects, so partial uploads that never finish do not cost money forever. The gateway still aborts sessions it knows are dead, and does not rely on lifecycle alone.

## Handing off to the pipeline

After the object is verified and recorded, the gateway starts an AWS Step Functions execution with a small input: the upload identity, the account, and the location of the source. The workflow takes it from there, running FFmpeg stages to build the adaptive-bitrate ladder and then packaging HLS.

The gateway must not start the workflow for a rejected upload. A 413 means no record that looks successful and no execution. If starting the execution fails after the object is stored, the upload is kept and marked as pending handoff, and a retry path picks it up, so the user does not have to upload again.

## Error handling and retries

Errors are split into client errors and server errors. Too large is a client error, answered with 413. Missing or bad credentials, bad types, and malformed requests are other client errors with their own statuses. Storage trouble, workflow trouble, and internal failures are server errors, and the client is told it is safe to retry.

The gateway should never answer success and then lose the file. When in doubt it fails loudly. Retries from the client reuse the upload identity so a repeat does not create a second copy or a second workflow run.

## Observability

Every rejection for size is logged with the account, the declared size if any, the point at which it was caught, and the request correlation field. A metric counts size rejections so operations can see if a publisher keeps hitting the cap, which usually means a workflow problem on their side worth a conversation.

Latency and error counts are tracked per stage: authentication, validation, storage write, record, and handoff. Logs must not contain upload contents or signed grant values.

## Testing notes

Tests should cover the boundary on both sides: an upload at the limit is accepted and one byte over is refused with 413. They should cover a client that lies about its size, a multipart set whose parts sum past the limit, and a direct to storage upload that ends up oversized. Each of these must leave no stray objects behind.

Use small synthetic stand-ins where possible, with the limit made configurable in tests, so the suite does not have to move huge files. One slow, opt-in test against real storage is worth keeping to confirm the genuine limit value is wired up correctly.

## Open questions

Should the limit be configurable per account for trusted publishers, or stay fixed? Current answer: fixed, because a single global rule is easier to explain and support. Revisit only if a real customer needs more and cannot proxy their source.

Should the gateway offer resumable uploads for slow connections? Probably yes, built on the multipart path, but the size rule stays the same. Also undecided is how long incomplete uploads are kept before they are cleaned up; that is a configuration choice and should be written down once chosen.

## Change log for this spec

First version records the size rule and the surrounding flow. Next edits should add the exact error body shape once it is agreed with the people who write the publisher-facing documentation, and should state the cleanup timing when it is decided. Any change to the size limit itself needs to update this note, the tests, and the operator-facing error text together.
