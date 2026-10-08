---
id: 01JVC9J447522AYF48S92Y7FQC
created: 2025-05-16T07:17-03:00
sources:
  - "code: crates/probe-analyzer/src/ffprobe.rs"
---

# probe-analyzer design

The probe-analyzer is the first step that looks inside an uploaded source file in ReelForge. It runs ffprobe with the arguments `-v error -print_format json -show_streams`, parses the JSON that comes back, and stores the parsed result as `probe.json` next to the source object in S3. Everything later in the pipeline (ladder planning, transcode, packaging) reads that file instead of probing the source again. This note records how it is built and why, written quickly so the next person does not have to rediscover it.

## Purpose

Media operations teams at independent publishers upload whatever they have: camera masters, screen captures, exports from editing tools. We cannot plan an adaptive-bitrate ladder without knowing the stream layout, codecs, resolution and frame rate of the source. The probe-analyzer answers those questions once and writes the answer down in one place.

## Where it runs

It is a Rust task invoked as a state in an AWS Step Functions workflow, right after the upload is validated. It receives the S3 location of the source object, runs ffprobe against it, and returns a short summary to the state machine. The full result lives in S3, not in the state payload, because Step Functions payloads are small and the stream list can be large.

## The ffprobe invocation

The arguments are `-v error -print_format json -show_streams`. Each part matters:

- `-v error` keeps stderr quiet unless something is actually wrong, so any stderr output can be treated as a signal.
- `-print_format json` gives machine-readable output we parse with serde instead of scraping text.
- `-show_streams` returns per-stream details. We do not ask for container-level format info in this invocation.

Do not add extra flags casually. Changing the output shape changes what downstream code sees in `probe.json`.

## Output file

The parsed result is stored as `probe.json` next to the source object, meaning in the same S3 prefix as the source. Keeping it beside the source makes it easy to find by anyone looking at the bucket, and lifecycle rules that remove a source also remove its probe result. The file holds our parsed structure, not the raw ffprobe text.

## Parsing

The Rust code deserializes ffprobe's JSON into typed structs. Fields we do not use are ignored. Fields that ffprobe sometimes omits are optional in the structs, since different containers report different things. Frame rates arrive as fractions in text form and are converted to a rational before anything compares them.

## Input access

ffprobe reads the object over a presigned S3 URL rather than a downloaded copy, so a large source does not have to land on local disk just to be inspected. If a source format needs to seek to the end to find its index, ffprobe handles it with range requests. Slow starts on some containers come from that behavior.

## Failure handling

A non-zero ffprobe exit, any stderr output under `-v error`, or JSON that does not parse is treated as a failed probe. The task fails with a clear reason and the workflow routes to the rejection path, so the uploader hears about a broken file early instead of after a long transcode. We do not retry on a parse failure, since the same file gives the same answer. We do retry on network errors reaching S3.

## Streams with no video

A file with no video stream is rejected at this step. Audio-only handling is out of scope for the current ladder logic. A file with several video streams is accepted, and the first one is treated as primary unless the ladder planner says otherwise.

## Downstream consumers

The ladder planner reads `probe.json` to choose rungs that do not upscale beyond the source. The transcode step reads it to select stream mappings for FFmpeg. The HLS packaging step reads it for language and track labeling. None of them call ffprobe themselves.

## Idempotency

Running the probe-analyzer again on the same source overwrites `probe.json` with the same content. This makes Step Functions retries safe. If the source object is replaced, the probe must be run again; nothing invalidates the old file automatically, so the upload flow always triggers a fresh probe.

## Security notes

Source files are untrusted input. ffprobe runs with no write access to the bucket, and the task's only write is the result file. Presigned URLs are short-lived and are not logged.

## Open questions

- Whether to add container-level info to the invocation for duration checks, which would change the arguments.
- Whether to cache probe results by content hash across re-uploads.
- Whether audio-only sources should become a supported path.

## Things to remember

The invocation and the file name are the contract. If either changes, check every downstream reader before shipping.
