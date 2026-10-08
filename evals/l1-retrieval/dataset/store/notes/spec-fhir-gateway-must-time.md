---
id: 01K7QDYY4155C662PTKS2XHFPG
created: 2025-10-16T18:14-03:00
---

# fhir-gateway upstream timeout spec

This note specifies how fhir-gateway behaves when it calls something upstream. The rule that matters most: fhir-gateway must time out upstream calls after 8 seconds. That limit applies to every outbound request the gateway makes to a FHIR server or to any other system it fronts for ClinicSlotter. A call that has not finished within 8 seconds is abandoned, counted as a failure, and handled by the rules below. Nobody should raise the limit locally to make one slow partner work. If a partner needs more time, that is a design discussion, not a config tweak.

The reason is the front desk. Staff at small clinics book appointments while a patient is standing or on the phone. A slot search that hangs for a minute is worse than one that fails quickly with a clear message, because the person can then pick another slot, retry, or book by hand. The limit also protects the Rails web workers on Heroku. A worker blocked on a slow upstream is a worker that cannot serve anyone else, and a small clinic deployment has few of them. Without a hard cap, one sluggish partner system can make the whole app look down.

## Scope of the timeout

The 8 seconds is the total budget for one upstream call, not a per-packet or per-read value. It covers connecting, sending the request, waiting for the response, and reading the body. If the HTTP client only supports separate connect, read and write settings, set them so that the sum of them cannot exceed 8 seconds, and prefer a single overall deadline when the client offers one. A slow trickle of bytes must not be able to keep a call alive past the deadline. This is the most common way a per-read timeout silently becomes a much longer wait.

What counts as an upstream call:

- Searches and reads against a FHIR server, such as looking up a patient, a practitioner, a schedule or a slot.
- Writes against a FHIR server, such as creating or updating an appointment resource.
- Any auxiliary request the gateway makes only to complete one of the above, for example fetching a capability statement or a token.

What does not count: work that happens inside ClinicSlotter itself, such as the scheduling logic that respects clinician availability and room constraints, database queries to MySQL, and Sidekiq job runtime in general. Those have their own limits. The 8 seconds is only about time spent waiting on something outside our process boundary through fhir-gateway.

The deadline starts when the gateway begins the call, not when the user clicked. If the gateway makes several upstream calls to answer one request, each call gets its own 8 seconds, but the caller of fhir-gateway is still responsible for deciding whether the whole operation is taking too long. Do not stack calls in a way that makes a single screen wait for a long chain of them. If a screen needs several lookups, run them in parallel or move them to a background job.

## Behaviour when the limit is hit

When a call passes the limit, fhir-gateway cancels the request on our side and raises one well-defined timeout error to its caller. The error should name the upstream target in general terms and say that the call timed out, so logs make it obvious that the cause was time and not a rejected request. It must not be mixed up with an upstream error response. A timeout means we do not know what happened on the other side, and callers must treat it that way.

This matters most for writes. If creating or changing an appointment times out, the upstream system may or may not have applied the change. The gateway must not report success, and it must not report a clean failure either. It reports an unknown outcome. The caller then has to check the upstream state before retrying, or use an idempotency approach so that a retry cannot book the same slot twice. Double booking a clinician or a room is the exact thing ClinicSlotter exists to prevent, so a blind retry of a timed-out write is a bug.

For reads, a timeout is safe to retry because nothing changes upstream. Retrying is still limited and is the caller's decision, not an automatic loop hidden inside the gateway. Keep these rules:

- The gateway itself does not retry a call that timed out. It returns the timeout to the caller.
- Callers on the web path may show the user a short message and offer a manual retry, and should not retry in a loop.
- Callers running in Sidekiq may retry with backoff, using the normal job retry mechanism, but only for reads or for writes that are safe to repeat.
- No code path may extend the wait by retrying inside the same user request.

The user-facing message should be plain. Something like: the other system did not answer in time, try again, or book this one manually. Do not show raw error text, stack traces, or patient data in that message. Patient identifiers and clinical details must stay out of timeout logs and error reports, because those go to third-party services. Log the target, the kind of operation, and the elapsed time, and nothing about the patient.

## Web path versus background jobs

The limit is the same everywhere, but the way a caller reacts differs.

On the web path, the user is waiting. A timeout should come back fast enough that the page can render an error state instead of hanging. Keep in mind that the router on Heroku has its own request limit, and our upstream deadline has to be well under it so that our own error handling gets to run before the platform kills the request. If anything ever makes the sum of upstream waits in one request approach the platform limit, the design is wrong and the work belongs in a job.

In Sidekiq, nobody is staring at a screen, so the pressure is different, but the cap still stays. A job that waits a long time on a dead upstream ties up a Sidekiq thread, and a few of those can starve the queue for unrelated work such as reminders. Keeping the same short deadline lets the job fail, release its thread, and be retried later on the queue's normal schedule. Jobs should be written so a rerun is safe, since a timeout can leave the outcome unknown, as described above.

Syncs that pull many resources from a partner should page their requests and treat each page as one call with its own deadline. Do not ask for a huge page just to reduce the number of calls, because a bigger response is more likely to run past the limit and then the whole sync fails instead of one small part of it.

## Configuration and testing

The value lives in one place in the gateway configuration and is read from there by every client the gateway builds. Do not hardcode a different limit in a caller, a job, or a test helper. If you find a second place that sets its own limit, treat it as a bug and fold it back into the single setting. The setting is not meant to be tuned per clinic. Small clinics share the same behaviour so that support can reason about it without checking each account.

Tests should cover the following cases, using a stubbed upstream that can be made slow on purpose and a fake clock where possible, so the suite does not actually sit and wait:

- A call that finishes just inside the limit succeeds and returns the data.
- A call that exceeds the limit raises the timeout error, and the request is cancelled.
- A slow trickle response, where bytes keep arriving but the whole thing takes too long, is also cut off at the limit.
- A timed-out write reports an unknown outcome, not success and not a plain failure.
- A timed-out read is not retried by the gateway itself.
- The error message and logs contain no patient data.

When reviewing a change that touches fhir-gateway, check three things quickly. First, did any new client skip the shared setting. Second, does any new write path assume that a timeout means the write did not happen. Third, does any new caller wait on several upstream calls one after another in a user request. Any of these is a reason to ask for changes.

## Open points

A few things are not settled and should not be guessed at in code. Whether some very heavy bulk operations deserve a different, explicit budget has not been decided; until it is, they use the same limit and are split into smaller calls instead. How the unknown-outcome state for writes is shown to front-desk staff in the interface needs a design pass with the people who use it. And we have not agreed on a shared way to detect that a timed-out write actually landed, for example by searching upstream for the appointment before any retry. Until these are written down as decisions, the rule stays simple: the limit is 8 seconds, the gateway reports a timeout honestly, and the caller decides what to do next.
