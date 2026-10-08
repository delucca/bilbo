---
id: 01KTW3Q6H73QEEH0K7NR2N2C32
created: 2026-06-11T16:49-03:00
---

# probe-analyzer fails on truncated MP4 uploads

The probe-analyzer fails on truncated MP4 uploads with the ffprobe error `moov atom not found`. The gateway must therefore verify that an upload is complete before anything is handed to probe-analyzer. If the gateway skips that check, a half-uploaded MP4 reaches the probe step, ffprobe reports `moov atom not found`, and the job fails there instead of at the door. Nothing is wrong with the analyzer itself in that case; the input file is incomplete.

## What happens

An MP4 keeps its index, the moov atom, in one place. When the file is cut short, that atom may never have been written, or it sits at the end of the file and was lost with the missing tail. ffprobe cannot read stream layout, duration or codec info without it. It exits with an error and the message `moov atom not found`. probe-analyzer wraps ffprobe, so it surfaces that same failure and produces no probe result for the file.

The symptom looks like a bad file or a codec problem. It is neither. It is almost always an upload that did not finish: a dropped connection, a client that gave up, or a multipart upload to AWS S3 that was never completed.

## Why the gateway has to check

The rule is simple: the gateway verifies upload completion first, then starts the workflow. probe-analyzer should not be the component that discovers a truncated file.

- A truncated file that gets through starts an AWS Step Functions execution that fails on its first real step. That wastes an execution and puts a confusing error in front of the media operations team.
- Retrying the probe step does not help. The file is still truncated on every attempt, so the same `moov atom not found` comes back each time.
- Later stages (FFmpeg transcodes, HLS packaging) never see the file, which is the right outcome. Failing early keeps the ladder output clean.

What counts as complete should be decided at the gateway: the object exists in S3, the upload has finished rather than being in progress, and the stored size matches what the client said it was sending. Keep that logic in one place at the gateway and do not copy it into probe-analyzer.

## Quick reproduction

Cut a valid MP4 short and probe it. The error shows up straight away.

```
ffprobe truncated.mp4
# error: moov atom not found
```

## What to do when you see it

1. Check whether the source object in S3 is the full size the uploader meant to send. Compare against the client's report or the original file.
2. If it is short, ask for a re-upload. Do not retry the workflow against the same object.
3. If the size is right and the error still appears, then it is a different problem, for example a file whose index was never finalized by the encoder that made it. Treat that as a separate case and do not assume the gateway check failed.
4. If truncated files are reaching probe-analyzer at all, the gateway completion check is missing or too loose. Fix it there.

## Notes for later work

- Do not add a workaround in probe-analyzer that tries to repair or guess at a missing index. The fix belongs upstream, at upload verification.
- It is fine for probe-analyzer to report the ffprobe error text unchanged. Keeping the original `moov atom not found` wording makes logs easy to search and makes the cause obvious to whoever reads the failed execution.
- If gateway behavior changes, for example a new upload path or a resumable upload mode, recheck that the completion check still runs before the workflow starts.
- Other container formats can fail differently. This note only covers truncated MP4 files and that one error.
