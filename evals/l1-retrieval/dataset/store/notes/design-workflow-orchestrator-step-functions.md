---
id: 01JRQ8HGHCQADHAAFX5JMS5E3V
created: 2025-04-13T06:43-03:00
sources:
  - "code: infra/state-machine.asl.json"
---

# workflow-orchestrator design

The workflow-orchestrator is an AWS Step Functions Standard state machine named `reelforge-transcode-v2`. Its Map state runs the rung encodes with `MaxConcurrency 8`, so at most eight rung encodes are in flight for one upload at any moment. Everything else in this note hangs off those two facts. If you change the state machine name or the Map concurrency, update this note first and then the rest of the pipeline.

## What it does

An uploaded source video lands in S3. The workflow-orchestrator is started for that upload and drives it through probing, ladder planning, the parallel rung encodes, packaging and publishing. It does not encode anything itself. Each step is a task that calls out to a worker, and the workers run FFmpeg. The workers are written in Rust.

The workflow-orchestrator is a Standard workflow, not Express. We picked Standard because encodes can run for a long time, because we want a full execution history per upload that an operator can read, and because a run must survive a worker restart without losing its place. Express would have been cheaper per run but gives neither of those.

The state machine is named `reelforge-transcode-v2`. The v2 is the second shape of the flow; the first one encoded rungs one after another and was far too slow on long sources. Anyone searching the console or the logs for executions should search for that name.

## Flow of states

The order is roughly this:

- Probe the source and read its duration, resolution, frame rate and audio layout.
- Plan the ladder. The planner decides which rungs make sense for this source. It never plans a rung above the source resolution.
- Map over the planned rungs. Each iteration encodes one rung.
- Package the encoded rungs into HLS: segments, per-rung playlists and the master playlist.
- Write the packaged output to the publish location in S3 and mark the upload as done.

The Map state is where the time goes. It runs the rung encodes with `MaxConcurrency 8`. A typical ladder has fewer rungs than that, so for most uploads every rung starts at once. The limit matters for long sources with a wide ladder, and for bursts when several uploads arrive together, because each execution has its own Map and the limits add up across executions.

## Concurrency notes

`MaxConcurrency 8` is a cap on iterations of one Map state, not a global cap. Total load on the encode workers is the sum over all running executions. If the worker pool is small, a burst of uploads can queue work on the workers even though each execution is within its own limit. Watch the pool, not only the state machine.

Raising the number gives faster ladders only if the workers have spare CPU. FFmpeg encodes are CPU heavy and a worker that is oversubscribed makes every rung slower, which can end up with the same wall time and more timeouts. Lower it if workers start timing out under load.

Do not set it to unlimited. Without a cap, a wide ladder on a big source would try to start every rung at once and would starve other uploads.

## Failure handling

Each encode task has its own retry with backoff for transient worker errors and S3 throttling. A hard failure on one rung fails the Map state, and the execution goes to a failure path that records which rung failed and why, so an operator can see it without opening FFmpeg logs first. We chose fail-the-whole-upload over publishing a partial ladder, because a partial ladder makes players switch to a bitrate that does not exist for part of the video.

Packaging is idempotent. If it is retried it overwrites the same keys in S3 rather than adding new ones. Encode outputs are written under a per-execution prefix so two runs for the same upload cannot mix segments.

## Open points

- Whether to split very long sources into time chunks inside the Map, so one slow rung does not hold the execution. Not decided.
- Whether the concurrency cap should vary with ladder width. For now it is fixed.
- Execution history retention for old uploads still needs a decision with the media operations teams who use ReelForge.
