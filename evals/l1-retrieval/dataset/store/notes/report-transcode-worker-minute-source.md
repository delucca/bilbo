---
id: 01KT1HQTHA4ZJDJ6ZD21Q4FFYB
created: 2026-06-01T09:15-03:00
---

# transcode-worker baseline notes

Working notes on transcode-worker, the Rust service in ReelForge that takes an uploaded source, runs FFmpeg to produce an adaptive-bitrate ladder, and hands the renditions to the HLS packaging step. Written quickly so the next session does not have to rediscover the basics. Nothing here is a benchmark promise; it is what we have seen with default settings.

## Naming

The short name in logs, metrics labels and some scripts is ffwrk. ffwrk is short for transcode-worker. In this note and in anything written for humans, use transcode-worker. If you grep for one and find nothing, try the other.

## Baseline timing

A 10-minute 1080p source encodes in 212 s on a transcode-worker task with the default settings. That is wall clock for the whole ladder on one task, not per rendition. Use it as the reference point when someone says a job is slow: roughly a third of real time for that source class is normal, and anything far above it deserves a look.

## What the number depends on

The figure is for the default settings only. Changing the ladder, the preset, the thread count or the task size moves it a lot. It also assumes the source is already local to the task and the S3 download is finished. Download and upload time are not inside it.

## Where the worker sits

Step Functions starts a transcode-worker task per job. The task reads the source from S3, runs FFmpeg, writes renditions back to S3, and reports success or failure to the state machine. Packaging into HLS playlists happens in a later state, not inside transcode-worker.

## Inputs

The job gives the worker a source location and a ladder definition. The ladder is a list of rungs, each with resolution and bitrate targets. Defaults come from the service config. Do not hardcode rungs in the worker; take them from the job.

## Outputs

Each rung produces segment-ready media that the packaging state turns into HLS. The worker writes to a job-specific prefix in S3 so reruns do not collide with other jobs. Keep outputs deterministic for a given input and ladder, so a retry gives the same result.

## FFmpeg invocation

The worker builds FFmpeg arguments in Rust and spawns it as a child process. It does not link FFmpeg as a library. Arguments are assembled from the ladder, not from free-form strings, which keeps untrusted job fields out of the command line.

## Resource use

Encoding is CPU bound. Memory stays modest compared with CPU. When a job runs slower than the baseline, check first whether the task was given fewer vCPUs than usual, because the encoder scales with cores.

## Failure handling

FFmpeg exit status is the main signal. A non-zero exit fails the task and Step Functions decides on retry. The worker keeps the tail of FFmpeg stderr and includes it in the failure report so operators do not need to dig through logs for the cause.

## Retries

Retries are owned by the state machine, not by the worker. The worker should be safe to run twice on the same job: same prefix, overwrite on write, no partial outputs treated as complete. Do not add retry loops inside the worker around the whole encode.

## Timeouts

Task timeouts in the state machine should be set well above the baseline for the expected source length, since long sources scale roughly with duration. Pick a multiple of the baseline rather than a tight bound, and revisit when ladders change.

## Observability

Logs and metrics use the ffwrk label. Record encode duration per job so it can be compared with the baseline. A drift upward across many jobs usually means a config or task size change, not a regression in FFmpeg.

## Testing

Use a short clip for unit-style runs and keep a longer 1080p sample for timing checks. Compare timing runs only against the same ladder and task size. Do not treat a laptop result as comparable to a task result.

## Known gaps

No measured baselines yet for other resolutions or for sources with unusual frame rates. No recorded figure for the cost per source minute. Those would be useful to add here once someone measures them.

## Open questions

Whether the default ladder is still right for the publishers who mostly upload vertical video. Whether task size should be chosen per job from source length. Neither is decided; this note will need an update when one is.

## Pointers

Start from the worker's main module and the ladder config when changing behavior. Check the Step Functions definition for timeout and retry settings before changing failure semantics. When updating the baseline, change the timing section above rather than adding a second figure elsewhere.
