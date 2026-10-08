---
id: 01KD7RWYT68EM7V3Z0CEA4NSSP
created: 2025-12-24T05:51-03:00
---

# ci-dispatcher fails with 401 Bad credentials when the installation token goes stale

The ci-dispatcher gets `401 Bad credentials` from the GitHub API when the installation token is older than one hour and has not been refreshed. The error looks like a permissions or configuration problem, but it is almost always an expired token that nobody refreshed. Check the token's age before you touch app permissions, secrets or repository settings.

This note covers what the failure looks like, why it happens, how to confirm it quickly, how to recover, and what to avoid when fixing it.

## What you see

The ci-dispatcher is the part of PatchPilot that triggers GitHub Actions runs for an upgrade pull request and then polls for the result. When the token is stale, the first call that needs authentication fails with `401 Bad credentials`. The response body is the usual short JSON message from GitHub, and the status is a plain unauthorized, not a forbidden.

The symptoms in PatchPilot are these:

- A dependency upgrade pull request is opened fine, but the targeted test run never starts. The pull request sits with no check attached.
- Or the run starts and the ci-dispatcher then fails while polling for status. The run finishes on GitHub, but PatchPilot never records the outcome.
- Jobs that worked in the morning fail in the afternoon with no deploy, no config change and no new secret in between.
- A restart of the process makes everything work again for a while, then it breaks again. This is the strongest hint that the token is the problem.
- Several repositories fail at about the same time, because they share the same long-running process and the same cached token.

The message says the credentials are bad. It does not say they are expired. GitHub returns the same text for a token that expired, a token that was revoked and a token that is malformed. So the error text alone cannot tell you which one you have. The age of the token can.

## Why it happens

The ci-dispatcher authenticates as a GitHub App installation. It signs a short-lived app credential, exchanges it for an installation token, and uses that token for API calls. An installation token is valid for one hour only. That lifetime is set by GitHub and cannot be extended from our side.

The trap is in the cache. The ci-dispatcher keeps the installation token in memory so it does not mint a new one for every call. If nothing checks the token's age before reuse, the cached value is used past its expiry. The first request after the hour is up gets `401 Bad credentials`.

A few things make this easy to miss:

- Short test runs hide it. A local run or a quick integration test finishes well inside the hour, so the token is always fresh.
- Long queues expose it. When many upgrade pull requests are queued behind one another, a job may wait long enough that its token expires between the time it was picked up and the time it calls GitHub.
- Polling loops expose it. A job that waits on a slow test run keeps polling long after the token it started with has expired.
- Process uptime matters more than job length. The token is shared across jobs, so a job that starts one minute before the token expires can fail even though the job itself is short.

There is also a related mistake: refreshing only on failure. If the code catches the first failure and then refreshes, one request is lost each time. For calls that are not safe to repeat, such as creating a check or dispatching a workflow, that can mean a duplicate or a gap. Refreshing before expiry is safer than refreshing after a failure.

## How to confirm it

Do these in order. They are cheap, and the first two usually settle it.

- Look at when the process last started, and at when the failing call was made. If the gap is longer than one hour and nothing refreshed in between, the token is stale.
- Look at the log line from the first failure. A stale token fails on the first call after expiry, so the earliest `401 Bad credentials` in the log marks the moment the token aged out. Everything before it should have succeeded.
- Check whether the failures stop after a restart. If a restart clears them, the cause is a stale token and not a missing permission.
- Check whether the failures hit every repository at once or only some. A stale shared token hits all of them. A permission problem on one installation hits only that installation's repositories.
- If you still doubt it, mint a fresh installation token by hand and make one authenticated call with it. If that call succeeds, the app, its key and its installation are fine, and the cached token is the only problem.

Do not paste a token into a ticket, a chat or a log while you do this. Treat any installation token as a secret even though it expires.

## What it is not

Several other problems produce the same status, and it is worth ruling them out only after the token age check fails to explain things.

- A wrong or rotated app private key. This fails every time, from the very first call after startup, not after an hour. If a fresh process cannot get a token at all, look at the key and the app identifier.
- An installation that was removed or suspended. This also fails from the start for the affected repositories, and the failure does not clear on restart.
- A personal access token set by mistake in place of the installation token. This can work for a while and then fail when the person's token is revoked or expires. It does not follow the one hour pattern.
- Clock skew on the host. If the machine's clock is far off, the signed app credential can be rejected as not yet valid or already expired. This fails at the exchange step, before any installation token exists, so the symptom differs: you never get a token to cache.
- Missing permissions. These normally return a forbidden status, not an unauthorized one. If you see a forbidden response with a message about a resource not being accessible by the integration, fix the app permissions. If you see `401 Bad credentials`, start with the token age.

The quick way to tell the cases apart is timing. A stale token works at first and fails later. The other causes fail at the very start or fail in a pattern unrelated to elapsed time.

## Fixing and recovering

To recover a running system right now, restart the ci-dispatcher so it mints a new installation token. Jobs that failed with `401 Bad credentials` did not run their tests, so they need to be re-queued. Check the state of each affected pull request before re-queuing. A job may have dispatched its workflow and failed only while polling, and then a blind re-queue would start a second run.

The permanent fix is to make the token's age part of every use:

- Store the expiry time next to the token when it is minted. GitHub returns the expiry with the token, so there is no need to guess from a local timer.
- Before each use, compare the stored expiry with the current time. Refresh when the token is close to expiring, with a safety margin so a request that is already in flight does not outlive the token. Keep the margin generous, because clock differences and network delay both eat into it.
- Make the refresh a single shared step. If many jobs notice an expired token at the same moment, only one should mint a new one and the rest should wait for it. Otherwise a burst of jobs makes a burst of token requests, which can run into rate limits of its own.
- Re-read the token for each call inside long loops. Do not copy the token into a local variable at the start of a job and keep using it while polling. A polling loop that captured the token once will fail even when the shared cache is healthy.
- Treat one `401 Bad credentials` as a signal to refresh once and retry once, for calls that are safe to repeat. Do not retry in a loop. If the retry with a fresh token also fails, the problem is not staleness, and the code should stop and report it clearly.
- For calls that are not safe to repeat, rely on the expiry check before the call and not on retry after it.

## Tests and monitoring

This bug passes ordinary tests, so it needs a test written for it on purpose.

- Use a fake clock in the ci-dispatcher tests. Mint a token, advance the clock past one hour, make a call, and assert that a new token was minted first. Without a fake clock the test would have to wait an hour.
- Add a case where the token expires in the middle of a long polling loop, and assert that the loop picks up the new token.
- Add a case with many concurrent jobs at the expiry boundary, and assert that only one refresh happens.
- Add a case where the refresh itself fails, and assert that the error is reported as a refresh failure and not as a generic `401 Bad credentials` from a later call. The two need different responses from whoever is on call.

For monitoring, log the token's expiry time at the moment of each refresh, and log which job triggered it. Do not log the token. Count `401 Bad credentials` responses as their own metric, separate from other client errors. A rise in that count with a flat count of refreshes is the signature of this bug coming back. Alert on it, because the failure is silent from the pull request's side: the upgrade just looks like it is waiting for checks.

## Notes for the next person

If you see `401 Bad credentials` from the ci-dispatcher, ask first how long the process has been running and when the token was last minted. Most of the time that answer closes the case.

Be careful with any change that adds a long-lived credential to avoid the refresh. It would remove this failure but would widen what a leaked secret can do, and it goes against the reason for using short-lived installation tokens in the first place. Fix the refresh instead.

If the same error appears right after a fresh start, stop looking at token age. Go to the key, the app identifier, the installation state and the host clock, in that order.
