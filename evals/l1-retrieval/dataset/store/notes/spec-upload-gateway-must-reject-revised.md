---
id: 01KQ7X25EEVXMV965PZ3EA0KCC
created: 2026-04-27T13:41-03:00
---

# upload-gateway size limit

The upload-gateway size limit is raised to 80 GiB, which replaces the earlier 50 GiB limit, because publishers upload long-form masters. This note replaces the earlier note "upload gateway must reject"; the new value is 80 GiB.

## Summary of the change

The maximum size of a single upload accepted by upload-gateway is now 80 GiB. Before this, the limit was 50 GiB. Anything at or under the new limit is accepted; anything over it is still rejected, as before. Only the number changed.

## Why the limit moved

Media operations teams at independent publishers send long-form masters: full-length films, event recordings, multi-hour broadcasts. Those files are mezzanine or near-lossless, so they are large. The 50 GiB limit was turning away legitimate masters, and publishers had to split or recompress them before upload. That is the reason for 80 GiB.

## What the old note said

The earlier note, "upload gateway must reject", described rejecting oversized uploads at the old 50 GiB value. The rejection behavior itself is unchanged. Treat that note as superseded and do not quote the 50 GiB figure as current.

## Current value

```text
upload-gateway max upload size: 80 GiB
previous value:                 50 GiB (replaced)
```

## Scope of the limit

The limit applies to the size of one uploaded source file arriving at upload-gateway. It is not a limit on the total of all uploads from a publisher, and it is not a limit on the size of transcoded outputs.

## Behavior at the boundary

A file exactly at the limit is accepted. A file above it is rejected before the full body is stored where possible, so the gateway does not spend storage and bandwidth on a doomed upload. If the size is only known after streaming, the upload is aborted once the count passes the limit.

## Rejection handling

Clients get a clear refusal that states the file is too large. The refusal should name the current limit so publishers know what to aim for. Nothing is handed to the pipeline for a rejected upload, and partial data is cleaned up.

## Storage implications

Accepted masters land in AWS S3. Larger objects mean multipart upload is required in practice, and the gateway must not buffer a whole file in memory. Check that lifecycle rules for incomplete multipart uploads still clean up abandoned large uploads.

## Pipeline implications

Uploads feed AWS Step Functions workflows that run FFmpeg transcodes in Rust-driven workers to build adaptive-bitrate ladders, then package to HLS. Larger sources mean longer transcodes. Worker disk, memory and timeouts should be checked against the bigger inputs rather than assumed to be fine.

## Timeouts

Long uploads of big files take a long time on slow links. Idle and total request timeouts on upload-gateway and any load balancer in front of it must allow for an 80 GiB transfer. Resumable or chunked uploads are preferred over single long requests.

## Client expectations

Publisher tooling and documentation that mention the old limit need updating. Any client-side pre-check that compares file size to a fixed number should use the new value. The server remains the authority; client checks are a convenience.

## Configuration

The limit is a single configured value in upload-gateway, not scattered through the code. Change it in one place, and keep any duplicate checks, such as proxy body-size settings, in step with it. A mismatch means the smaller one wins and users see a confusing rejection.

## Testing

Test a file just under the limit, a file exactly at it, and a file just over it. Confirm the over-limit case is refused with the clear message and leaves no stray objects in S3. Simulated sizes are fine; a real 80 GiB file is not needed for every run.

## Monitoring

Watch rejection counts after the change. A drop in too-large rejections is expected. Also watch transcode duration and failure rates for very large sources, since those are the new load.

## Cost

Storing and transcoding bigger masters costs more in S3 and compute. Publishers uploading near the limit will notice it in usage. This is accepted as the price of supporting long-form content.

## Rollback

If the larger limit causes trouble downstream, lowering the configured value is the rollback. Uploads already accepted are not affected by a later lowering. Do not go back to the old note's wording; record any new value in this note.

## Open questions

Whether the limit should vary per publisher is not decided. For now one value applies to everyone. Revisit if a publisher needs more than the current value or if worker capacity becomes the bottleneck.
