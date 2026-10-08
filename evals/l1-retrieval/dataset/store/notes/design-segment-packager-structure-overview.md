---
id: 01KN6KXW5YQVBM8EDQ8242TP5A
created: 2026-04-02T05:11-03:00
---

# segment-packager structure

segment-packager is the stage in ReelForge that takes transcoded renditions and turns them into something a player can stream over HLS. It does not decide the ladder and it does not do the heavy encoding. It receives finished or near-finished media for each rendition, cuts it into segments, writes playlists, and puts everything where the delivery side expects it. This note describes how the component is laid out, not how it is tuned.

It is written in Rust and shells out to FFmpeg for the parts where FFmpeg is the right tool. It reads from and writes to AWS S3, and it runs as a step inside an AWS Step Functions workflow. Media operations teams at independent publishers are the people who feel it when it breaks, so most of the design leans toward being easy to rerun and easy to inspect.

## Where it sits in the pipeline

Upstream is the transcode stage. That stage produces one encoded output per rung of the adaptive-bitrate ladder. Downstream is whatever publishes the playlists and serves them. segment-packager sits between the two.

The Step Functions state machine invokes segment-packager once per job, or once per rendition when the workflow fans out. The state machine passes a small JSON payload that points at input objects in S3 and says where output should go. The component does not discover work on its own. If the workflow does not hand it something, it does nothing.

## Main parts

The code is split into a few pieces that mostly map to the order of work:

- an input resolver that turns the payload into a list of S3 objects and checks they exist
- a probe step that reads stream facts from each input
- a segmenter that drives FFmpeg
- a playlist builder that writes the media playlists and the master playlist
- an uploader that pushes results to S3
- a reporter that returns a result document to Step Functions

Keeping these separate matters because the playlist builder and the resolver are pure Rust and easy to test without FFmpeg. The segmenter and uploader are the parts that touch the outside world.

## Input resolution

The resolver takes the payload and builds a typed job description. Bad payloads fail here, early, with a message that names the missing or malformed field. It also checks that each referenced input object is present before any work starts, so a missing rendition shows up as a clear failure and not as a half-written output tree.

Inputs are streamed or staged to local scratch depending on what the segmenter needs. Where possible we avoid pulling whole files to disk, but FFmpeg is happier with seekable input, so staging is the common path.

## Probing

Before segmenting, the probe step reads codec, timing, and stream layout from each input. The results feed two things: the segmenter settings and the master playlist attributes. Probing is also where we catch inputs that do not line up with each other, for example renditions whose timing structure differs enough that segment boundaries would not match across the ladder.

Boundary alignment across renditions is the property that makes adaptive switching work cleanly, so the component treats a mismatch as a failure to report, not something to paper over.

## Segmentation

The segmenter builds an FFmpeg invocation for each rendition and runs it as a child process. Output is a series of media segments plus a playlist FFmpeg itself produces. We use FFmpeg's HLS muxing for the cutting and then treat its playlist output as raw material, not as the final artifact.

The Rust side owns process lifecycle: start, capture stderr, enforce a time limit, kill on cancel, and map exit status to a typed error. FFmpeg's own log output is kept as part of the failure context so someone on call can read it later.

Segments are cut on keyframe boundaries that the transcode stage already arranged. The packager is not supposed to re-encode. If it ever has to, that is a sign something upstream is wrong.

## Playlists

The playlist builder reads what the segmenter produced and writes the final playlists. There is one media playlist per rendition and one master playlist that lists them. The master playlist is built from probe data plus ladder metadata from the payload, which is why probing happens first.

Doing this in our own code instead of trusting FFmpeg's master output gives us control over ordering, naming, and relative URIs. Relative URIs matter because the same output tree gets served from different hosts and paths.

## Output layout in S3

Output goes under a job-scoped prefix. Each rendition gets its own subtree for segments and its media playlist, and the master playlist sits at the top of the job prefix. The layout is deterministic from the job description, so reruns land on the same keys.

Deterministic keys are the basis for idempotence. If a step is retried by Step Functions, it overwrites the same objects with equivalent content and no stale files from a failed attempt are left pointing anywhere.

## Upload ordering

Segments are uploaded first, media playlists next, and the master playlist last. A player or a publisher process that sees the master playlist can then assume everything it references already exists. This ordering is a core part of the design and is easy to break by parallelizing uploads carelessly.

Uploads run concurrently within a stage but the stages are sequenced. Failures in any upload fail the stage, and the reporter says which keys were affected.

## Errors and retries

Errors are split into two groups. Transient ones, such as S3 throttling or a network blip, are retried inside the uploader with backoff. Everything else is surfaced to Step Functions as a failure with a typed cause, and the state machine decides about retrying the whole step.

We keep the in-process retry narrow on purpose. Wide retry logic in the component hides problems and makes the workflow history less useful. FFmpeg failures are not retried inside the component at all.

## Observability

The component logs structured lines with the job identifier attached, so a single job can be followed across the transcode and packaging steps. The result document returned to Step Functions includes the list of produced playlists and a short summary per rendition. Operators use this to confirm that a job produced the whole ladder.

A minimal view of the data flow, using only the pieces named above:

```text
Step Functions -> segment-packager -> S3 (segments, media playlists, master playlist)
                      |
                      +-> FFmpeg (child process per rendition)
```

## Things to watch

- Do not reorder uploads so the master playlist can land before its segments.
- Do not let the playlist builder depend on FFmpeg being installed; keep it testable alone.
- Scratch space on the host is finite, so staged inputs need cleanup on both success and failure paths.
- Any change to key naming affects reruns and anything downstream that reads the tree.
- If a feature request needs re-encoding inside segment-packager, push back and look at the transcode stage first.

## Open questions

Some things are not settled and are worth revisiting when someone is in the code: whether staging inputs to disk can be dropped for more renditions, how much of the probe data should be passed through the workflow instead of recomputed, and whether the reporter should carry more detail for operators who do not have log access.
