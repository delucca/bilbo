---
id: 01KDTGRXFAKNZB3PA171E4TKHS
created: 2025-12-31T12:35-03:00
---

# upload-gateway fails with SignatureDoesNotMatch when client clock is skewed

Uploads through the upload-gateway fail with the S3 error `SignatureDoesNotMatch` when the client clock is skewed by more than 15 minutes. The error looks like a credentials or signing bug, but the keys and the signing code are usually fine. The cause is the device clock. Check the clock before you touch anything else.

This note is for anyone on media operations or on the ReelForge side who sees an upload die right at the start, or on the first part of a multipart upload, with a signature error. It also covers what to tell the person who uploaded.

## Symptom

The user picks a source file in the upload tool and starts the upload. It fails almost at once. No progress, or only a tiny bit, and then an error. The error that comes back from S3 through the upload-gateway is `SignatureDoesNotMatch`. Some clients show only a generic failure message, and you have to read the gateway logs or the raw response body to see the S3 code.

Things that make it look like something else:

- It affects one machine or one person, not everybody. Other editors upload fine at the same time.
- It does not depend on file size. A tiny test file fails the same way as a big mezzanine file.
- It started suddenly, with no deploy and no config change. Often the machine was just woken from sleep, restored from a snapshot, moved to another time zone by hand, or had its battery die.
- Retrying does not help. Retrying the same request again and again gives the same error every time, because the clock is still wrong.
- The same credentials work from another machine.

Because the message says the signature does not match, people first suspect a rotated key, a wrong secret, a bad bucket policy, or a proxy that rewrites headers. Those are all possible, but they are not the first thing to check.

## Why it happens

The upload-gateway hands the client a way to write to S3, and the request that reaches S3 is signed. A signed S3 request carries a timestamp that is part of what gets signed. S3 compares that timestamp with its own time. If the two are too far apart, S3 refuses the request. The tolerance is 15 minutes. Past that, the request is rejected, and the rejection can surface as `SignatureDoesNotMatch` instead of a clearer message about time.

So the failure is about the client clock, not about the secret. The client signs with its own idea of the current time. If that idea is wrong by more than 15 minutes in either direction, ahead or behind, S3 says no.

Two details that are easy to miss:

- The skew can go either way. A clock that is slow fails just as a clock that is fast does.
- The check is on the time the request is signed, so it is the client clock at signing time that matters. A clock that is fixed after the failure needs a fresh attempt, not a resume of the old signed request, since that request still carries the bad timestamp.

The gateway itself runs on servers with synced clocks, so server-side skew is not the usual cause. It is still worth a quick look if every client fails at once, which would point at the signing side instead of one client.

## How to confirm it

Start with the cheapest check: ask the user what time their machine shows, and compare it with the real time. A gap of more than 15 minutes is the answer. Do not trust the time zone display alone. A machine can show a plausible local time and still have the wrong underlying time because the zone was set to compensate for a wrong clock.

Then, if you have access to logs:

- Find the failed request in the upload-gateway logs and look at the S3 error code. If it is `SignatureDoesNotMatch`, note the time the request was received on the server.
- Compare that with the time the client claims for the request, if the client logs it. A large gap confirms skew.
- Look for a pattern. If the failures come from one client address or one user and everyone else is clean, skew on that client is the likely cause.

If the clock turns out to be right, the error is a real signing or credential problem, and you should go on to the usual suspects: the secret in use, the bucket and region the request was signed for, the path or query handling in any proxy between the client and S3, and whether the object key was changed after signing.

A quick test for the user: have them sync the clock and try a small file. If it works, that was the cause.

## What to do about it

For the person hitting the error, the fix is to correct the clock and upload again with a fresh attempt.

- Turn on automatic time setting in the operating system, so it syncs against a network time source.
- If it was already on and the time is still off, force a sync, then check that the machine can reach its time source. Locked-down office networks sometimes block it.
- On virtual machines, check that the guest clock follows the host. A paused or resumed VM is a common source of large drift.
- On laptops that were asleep for a long time, the clock may take a while to catch up after wake. Wait for it, or force a sync.
- Start a new upload after the clock is right. Do not try to resume the failed one.

For the ReelForge side, a few things that help without changing the signing model:

- Make the upload tool show a clear message when it sees `SignatureDoesNotMatch`, something like: check that your computer clock is correct. Right now the raw S3 error leaves people lost.
- Have the gateway log the S3 error code and the client-reported time next to each other, so support can see skew right away.
- Consider having the gateway return its own current time in the response when an upload fails with this error, so the client can show the difference. That is a suggestion, not something that exists today.

None of this loosens the tolerance. That limit belongs to S3, not to ReelForge, and we cannot change it from our side.

## What not to do

Things people tried that wasted time or made it worse:

- Rotating the access keys. The keys were fine. It broke other uploads that were working, and the failing client still failed.
- Rewriting or loosening the bucket policy. Policy errors give access denied style errors, not a signature mismatch from clock skew. Changing the policy only widens access for no gain.
- Retrying in a loop in the client. With a bad clock every retry fails the same way, and the loop only fills the logs and makes the user wait.
- Blaming the network or a proxy first. It can be the cause in other cases, but check the clock before taking a proxy apart.
- Pointing a downstream job at it. The Step Functions workflow and the transcode stage never see these uploads, since the file never lands in S3. If the pipeline looks idle for a given upload, check whether the upload failed at the gateway before you dig into the workflow.
- Changing the signing code to pad the timestamp. A fudge in the signing code hides the real problem for one machine and breaks it for the correct ones.

## Notes for later

What is still open or worth checking when this comes up again:

- Whether the gateway can detect likely skew itself and answer with a clearer error than the raw S3 one. The right place is probably where the gateway already maps S3 errors to client responses.
- Whether the upload tool should compare its clock against the server at start-up and warn early, before anyone picks a large file and waits.
- Whether support has a short runbook line for this. It should say: ask for the machine time first, compare it with real time, and only then look at keys and policy.
- Whether other signed operations in the pipeline have the same weakness. Anything where a client signs its own requests with its own clock can fail the same way, including downloads of finished packages if those are ever signed on the client.

The short version for a hurried reader: `SignatureDoesNotMatch` on an upload-gateway upload, plus a client clock off by more than 15 minutes, means fix the clock and start a fresh upload. Check this before you look at keys, policies or proxies.
