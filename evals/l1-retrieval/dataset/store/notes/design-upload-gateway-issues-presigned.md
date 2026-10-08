---
id: 01KRGM8B7Q7SPD178VD3H9KGEJ
created: 2026-05-13T09:16-03:00
sources:
  - "code: crates/upload-gateway/src/multipart.rs"
---

# upload-gateway design

The upload-gateway is the front door for raw video in ReelForge. Clients ask it for presigned S3 multipart upload URLs, then send the file straight to S3 without passing bytes through the gateway. It targets the bucket `reelforge-ingest-raw` and uses a part size of `16 MiB`. This note records the shape of the component and why it is built that way. Details not listed here are deliberately left general.

## Naming

The component used to be called `ingestor-v1`. It is called `upload-gateway` now. Old docs, dashboards, alarms and chat threads may still say `ingestor-v1`; treat that as the same thing. Do not treat it as a separate service, and do not create anything new under the old name.

## Purpose

Media operations teams at independent publishers upload source video that ReelForge later transcodes into adaptive-bitrate ladders and packages as HLS. The upload-gateway only handles the first step: getting the original file safely into storage and telling the rest of the pipeline it arrived. It does no transcoding itself.

## Why presigned multipart

Source files are large and uploaders are often on poor connections. Multipart upload lets a client retry one part instead of the whole file. Presigned URLs keep AWS credentials away from clients and keep large payloads off our own hosts. The gateway stays small and cheap because it never touches the video bytes.

## Target bucket

All uploads land in `reelforge-ingest-raw`. That bucket is for untouched originals only. Transcoded renditions and HLS packages go elsewhere. Keeping raw input in its own bucket makes lifecycle rules, access policy and cost tracking simpler.

## Part size

Every multipart upload uses a part size of `16 MiB`. The gateway decides this, not the client, so the URLs it hands out all match. A fixed size keeps part counting predictable, and the number of parts for a given file is easy to compute. If the size ever changes, existing in-flight uploads must be allowed to finish under the old value.

## Request flow

A client authenticates and asks to start an upload. The gateway creates the multipart upload in S3, works out how many parts are needed from the file size, and returns one presigned URL per part. The client uploads parts in any order, possibly in parallel, and then asks the gateway to complete the upload.

## Completion

On completion the gateway gathers the part identifiers returned by S3 and finishes the multipart upload. Only after S3 confirms the object exists does the gateway signal the pipeline. It must never signal on a partial or unconfirmed object.

## Handoff to the pipeline

The finished upload starts an AWS Step Functions execution that drives transcoding with FFmpeg workers written around Rust services. The gateway passes only a reference to the object, not its contents. Everything downstream reads from the raw bucket by that reference.

## Failure and retries

Part uploads can fail and be retried by the client using the same presigned URL until it expires. If a client abandons an upload, the incomplete multipart upload would keep costing storage, so a cleanup rule on the bucket should abort stale ones. Completion calls should be safe to repeat, so a client that lost the response can ask again.

## Expiry of URLs

Presigned URLs are short lived. A client that is slow may need to ask the gateway for fresh URLs for the remaining parts. The gateway should be able to reissue URLs for an upload that is still open without starting over.

## Security

Clients get no direct S3 permissions. Each URL is scoped to one part of one upload. The gateway checks who is asking before issuing anything, and it should not hand out URLs for objects owned by another publisher.

## Observability

Useful signals are uploads started, uploads completed, uploads abandoned, and time from start to completion. Logs should carry an upload identifier but never the presigned URLs themselves, since a URL is a credential while valid.

## Known gotchas

Old references to `ingestor-v1` can send people looking for a service that no longer exists. Multipart uploads left open are easy to forget and quietly cost money. Clients that assume their own part size will break against the fixed `16 MiB`.

## Open questions

Whether very large files need a different part size is undecided. Whether the gateway should verify checksums of parts before completion is also open. Resumable uploads across sessions need a clear owner.

## Related components

Downstream are the Step Functions workflow and the FFmpeg transcoding stage, then the HLS packaging stage. The upload-gateway has no dependency on them beyond starting the workflow.
