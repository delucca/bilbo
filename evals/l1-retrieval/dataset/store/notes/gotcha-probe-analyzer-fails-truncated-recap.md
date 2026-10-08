---
id: 01M0PZP91KSXGQ6ZENGJEZ0E6W
created: 2026-08-23T06:37-03:00
---

# probe-analyzer fails on truncated uploads

Notes written in a hurry, partly from memory of what we saw, so treat the gaps as gaps. The short version: when an uploaded file is cut off before the end, probe-analyzer does not always fail the way you would expect. Sometimes it fails loudly and the Step Functions execution goes red. Sometimes it passes with a half-filled result and the damage shows up a few stages later, in the ladder planning or in the packaging step, where the error looks like it has nothing to do with the upload. This is the thing to check first when a job dies somewhere downstream and the source looks fine in the bucket listing.

I did not look at older notes before writing this one, so some of it may repeat what is already recorded about the same failure. There is a related note on the ladder side, [[ladder-planner-targets-vmaf-revised]], which matters only because the planner consumes whatever probe-analyzer hands it.

## What a truncated upload looks like

A truncated file here means the object in S3 exists, has a plausible size, and the container header is intact, but the media data stops before the duration the header claims. Most often this comes from a browser or client that gave up partway through a multipart upload and the publisher's tooling marked it complete anyway. It also happens when a publisher copies a file out of a capture device while it is still being written. The object lands in the bucket, the upload-complete event fires, and the pipeline starts.

What makes this annoying is that the file is not garbage. The first part plays. A person opening it in a desktop player sees video, then it stops early or just freezes on the last frame. Media operations people at the publishers report it as "the video is fine" because the beginning is fine. So the support conversation tends to start from a wrong premise, and it is worth asking early whether the file was fully uploaded and whether the source machine finished writing it.

There are a few flavors, and they behave differently in the analyzer:

- The container index lives at the end of the file and the end is missing. This is the common case with some container families. The header says nothing useful, the probe has to scan, and the results are partial or empty.
- The index lives at the front, so the probe happily reads the declared duration and stream layout, but the actual data is short. The probe looks healthy. The failure is deferred until something tries to read the whole thing.
- The file is cut mid-packet. The probe reads up to the cut and reports a last packet that is malformed. Depending on how we treat warnings, this is either ignored or promoted to a failure.
- The audio stream is shorter than the video stream, or the other way round, because the two were interleaved unevenly before the cut. The probe reports both durations and they disagree by a lot.

Each of these needs a slightly different check. The first and the third usually produce a visible problem right at probe time. The second and the fourth are the sneaky ones.

## How probe-analyzer reacts

probe-analyzer is the Rust service that wraps the FFmpeg probe tooling, parses its structured output, and produces the summary the later stages use: stream layout, declared and measured duration, frame rate, resolution, codec info, and the flags the planner reads. It runs as a task inside the Step Functions state machine, right after the upload event is validated and before the ladder planner is invoked.

The behavior, as I understand it from reading the failure cases rather than from a clean spec:

- If the probe process exits with a failure status, probe-analyzer treats that as a hard failure and the state machine takes the failure branch. This covers the severe case of a missing index where nothing can be read.
- If the probe process exits cleanly but prints warnings about the stream, probe-analyzer logs them and carries on. This is where truncation slips through. A file cut mid-packet often exits cleanly with a warning.
- If the structured output parses but some fields are absent, the Rust side fills them with defaults in a couple of places instead of failing. I am not certain which fields. Duration is the one that bit us. A missing duration was turned into an empty value that later code read as zero length, and the planner then built a ladder for a clip that, as far as it knew, did not exist.
- If the output does not parse at all, the analyzer fails with a parse error. That error message is unhelpful, because it describes the JSON shape and not the media. People see it and go looking for a bug in our parser.

So the same root cause gives at least three different symptoms: a clean failure, a parse failure that points the wrong way, and a silent pass with a bad summary. When triaging, do not trust the symptom to tell you the cause.

## Where the failure surfaces downstream

The silent pass is the part worth remembering, because it moves the failure to a place where nobody thinks to look at the source.

The planner reads the probe summary and decides which rungs of the adaptive ladder to produce. If duration is wrong or the resolution fields are partial, it may select rungs that make no sense, or produce an empty ladder. Neither of these is flagged as an input problem. The planner's own notes cover the target logic; see [[ladder-planner-targets-vmaf-revised]] is the place for how targets are chosen. What matters for this note is that the planner does not re-validate what the analyzer told it. It trusts the summary.

If the planner does produce a ladder, the transcode tasks then read the actual file with FFmpeg. A file shorter than declared makes those tasks finish early, or stall waiting on data that never comes, depending on the container. When they finish early, the output renditions are shorter than the planned duration. The HLS packaging step then builds playlists whose segment counts do not match across renditions. Players handle that badly: some rungs end before others, and the player stalls or drops quality at the point where the shortest rendition stops. That is the version publishers notice, because the stream plays for a while and then misbehaves.

So the chain is: truncated source, analyzer passes with a thin summary, planner trusts it, transcodes produce short outputs, packaging produces mismatched playlists, the player breaks. Several stages between the cause and the visible symptom. Anyone starting from the symptom will spend a long time in the packaging code.

## What I would check first

In rough order, when a job looks wrong and nobody knows why:

- Compare the object size in S3 with what the publisher says the file should be. If they can tell you the size on their disk, this settles it quickly.
- Look at the probe-analyzer log for the execution. Warnings about the end of the stream, a last packet, or an unexpected end of data are the signature. They are logged at a low level, so make sure you are not filtering them out.
- Compare declared duration with measured duration in the summary, if both are present. A large gap is the strongest single indicator. A small gap is normal and should not trigger anything.
- Check whether audio and video durations agree. A mismatch beyond the usual tolerance suggests an uneven cut.
- Check whether any summary field that should be present came out as a default. If the duration is empty or zero, assume truncation or a failed read until proven otherwise.
- Only then go downstream and look at the planner and packaging.

The point of this order is that the early checks are cheap and the last is expensive. I wasted real time starting at the packaging end.

## The shape of the pipeline, for orientation

Just to keep the stages straight when reading logs. This is only the order, nothing more:

```
AWS S3 upload -> probe-analyzer -> ladder planner -> FFmpeg transcode -> HLS packaging
```

The state machine in AWS Step Functions drives all of it. probe-analyzer is the only stage that looks at the source as a whole before any expensive work is committed, which is why it is the right place to catch this. Everything after it assumes the source is sound.

## Fix ideas and open questions

None of this is implemented as far as I know. These are things I would try, roughly in order of how cheap they are.

First, make the analyzer stricter about the clean-exit-with-warning case. If the probe reports an end-of-data or malformed final packet warning, treat it as a failure for the input, with a message that says the source looks truncated. That message should name the problem in media terms, not parser terms. The cost is that some files with harmless trailing garbage would be rejected, so there should probably be a tolerance, a configured limit on how much missing tail is acceptable, rather than an all-or-nothing rule. I do not know what value that limit should have and I do not want to guess here; it should be set with the media operations people, who know what their capture tools produce.

Second, stop turning missing fields into defaults for the fields the planner depends on. A missing duration should be an error, not an empty value. In Rust this is the kind of thing that is easy to fix by making the field required in the parsed type, so a missing value fails at parse time with a message that names the field. The parse error is then at least about the right thing. This is a small change but it changes behavior for files that currently get through, so it needs a pass over recent jobs to see what would have been rejected.

Third, add a cross-check between declared and measured duration, using the usual tolerance, and fail or flag when they disagree by more than that. Measuring requires actually reading packet timestamps, which costs time on large files. It may be worth doing only when the cheap checks look suspicious, or only reading the tail of the file. I have not measured how expensive the full scan is. For very long source files it could be significant, so this needs a look before it goes into the default path.

Fourth, consider a check on the S3 side before probing at all: compare the object size against what the uploader declared, when the uploader declared anything. Many clients do send an expected size. If we already receive it with the upload event we can reject early and cheaply. If we do not receive it, then it is a change to the intake contract and that is a bigger conversation.

Fifth, make the planner defensive too. It should refuse to plan from a summary with missing or inconsistent duration and say so, rather than building a ladder from nothing. This is belt and braces, since the analyzer should have caught it, but it would have turned a multi-stage mystery into a single-stage one.

Open questions I could not answer:

- Whether the silent pass depends on the container family. My impression is that it does, and that the index-at-the-end formats fail loudly while the index-at-the-front formats pass quietly. I have not tested this properly, so do not rely on it.
- Whether the analyzer's retry behavior in the state machine makes things worse. If a failure is retried automatically, a truncated file gets probed again and fails again, which wastes time but is harmless. If the failure were treated as transient and retried many times, it would delay the error reaching the publisher. I did not check the configured retry policy for this task.
- Whether the publishers can re-upload easily. For some of them the original is gone, and a truncated source is all that exists. In that case the right behavior might be to proceed with what is there and say so, instead of rejecting. That is a product decision and not something to settle in the analyzer.
- How often this happens. It felt common while debugging, but that is the bias of looking at failures. There may be a way to count from logs: how many executions produced the warning and still passed.

## Things that look like this but are not

A few other failures produce similar downstream symptoms and are worth ruling out so nobody blames truncation for everything.

- Variable frame rate sources can give a measured duration that differs a little from the declared one, with no truncation at all. The gap is small, and it is within the usual tolerance. Do not flag these.
- Sources with a long silent or black tail look short in a player when the viewer stops at the last visible content, but the file is complete. The probe agrees with the header here.
- Files with an edit list or a start offset can report a duration that includes or excludes the offset depending on which field you read. The disagreement is not an upload problem. Make sure the comparison uses fields that mean the same thing.
- Packaging mismatches can also come from a transcode task that was killed or timed out while the source was fine. In that case the probe summary is healthy and the source size matches. The signal is on the transcode side, in the task logs, not in the analyzer.
- A bad rendition ladder from the planner can come from its own target logic and not from the input at all. That is the planner's business, not this note's.

The quick way to tell truncation from the rest is that truncation shows up at probe time if you look for it. If the probe log is clean and the sizes match, it is something else.

## Practical notes for whoever picks this up

Keep the user-facing message about this honest. Publishers read errors that mention parsing or JSON as our fault. A message saying the source file appears to be incomplete and should be uploaded again gets the right action from them, and cuts down the back and forth with support.

When changing the analyzer, keep a few small truncated sample files around for tests: one cut in the middle of the data, one with the tail removed so the index is missing, one with uneven audio and video. Make them from real source files by cutting the tail, not by hand editing, and keep them small so they do not slow the test suite. The tests should assert on the category of failure, not on the exact wording of the probe output, since that wording changes between FFmpeg releases and would make the tests brittle.

If you change which warnings count as failures, tell the media operations contacts before it ships. Some publishers may currently be getting acceptable output from slightly damaged files, and a stricter rule will start rejecting them. That is probably the right call, but they should hear it from us first and not find out when a job fails.

This note is deliberately loose. If someone has the exact failing cases written down elsewhere, those win over what is here, and this should be folded into that note instead of kept separately.
