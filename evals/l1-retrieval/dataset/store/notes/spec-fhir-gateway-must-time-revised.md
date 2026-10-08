---
id: 01KXSPD3DWATDVX3FHQYS9A3AB
created: 2026-07-18T01:05-03:00
---

# fhir-gateway upstream timeout spec

This note replaces the earlier note about "fhir gateway must time". The new value is that fhir-gateway must time out upstream calls after `5 seconds`, which replaces the earlier 8 second timeout. Anything that still says 8 seconds is out of date and should be changed or deleted when found.

The rest of this note says what the limit covers, what happens when it fires, how it interacts with retries and Sidekiq, and what to check before calling the change done. It is written quickly, so treat the intent as firm and the implementation details as general. Where a detail is not pinned down, the note says so.

## Requirement

fhir-gateway is the part of ClinicSlotter that talks to outside HL7 FHIR servers. Front-desk staff at small clinics use the scheduler while a patient is standing or sitting in front of them or while they are on the phone. A slow upstream makes the whole screen feel stuck. The old limit was too generous for that. The point of the shorter limit is that a stuck upstream should be given up on quickly enough that the person at the desk can carry on, either with the data we already have or with a clear message that the outside system did not answer.

The rule, stated so this note can be read alone: every call that fhir-gateway makes to an upstream FHIR server must be abandoned if it has not finished within `5 seconds`. This is the only timeout value the spec sets. It replaces the earlier 8 second value everywhere that value was used.

Things that follow from the rule:

- The limit applies to each upstream call as a whole, not only to one phase of it. If the connection is slow to open, slow to send, or slow to answer, the clock keeps running against the same budget.
- The limit is a ceiling. A call that finishes sooner is fine and nothing should wait out the remainder.
- The value should live in one place in configuration or in one constant, so that changing it later is a one-line change. It should not be repeated as a literal in several clients.
- No caller should quietly pass a longer value for its own convenience. If a caller thinks it needs more time, that is a conversation about the spec, not a local override.

## What the limit covers

The limit covers calls that fhir-gateway makes outward to FHIR servers. These include reads of patient, practitioner, schedule, slot and appointment resources, searches over those resources, and writes that create or update an appointment on the upstream side. It also covers any capability or metadata lookup that fhir-gateway does against an upstream before it uses it.

It does not cover calls coming into ClinicSlotter from the browser or from other parts of the Rails app. Those have their own limits set by the web server and by Heroku, and nothing in this spec changes them. One thing to keep in mind is that the Heroku router has its own cut-off for web requests. The upstream limit here has to stay comfortably under whatever budget a web request has, so that a timed-out upstream call can still be turned into a proper response instead of the router killing the request first. If someone ever proposes raising the upstream limit, they must check this relationship first.

It also does not change how long a background job as a whole may run. A Sidekiq job that makes several upstream calls can take longer than a single call limit, because each call has its own budget. The limit is per call, not per job.

Pagination deserves a note. A search that returns results in pages makes one upstream call per page. Each page fetch gets its own limit. If one page times out, the search as a whole is treated as failed unless the caller has said it can accept partial results. Nobody should assume that a partial list is complete.

## Behavior when the limit is hit

When an upstream call exceeds the limit, fhir-gateway abandons it and reports a timeout to its caller. The timeout must be a distinct, recognizable kind of failure, not a generic error and not an empty result. A caller has to be able to tell the difference between the upstream said there is nothing and the upstream did not answer. Mixing these up is the worst outcome here, because in scheduling an empty answer reads as free time, and a clinician could be double booked.

Rules for the failure path:

- A timed-out read must never be treated as an empty result. For availability data in particular, a timeout means unknown, and unknown must not be shown as open.
- A timed-out write is the hard case. The upstream may or may not have applied the change when we gave up. fhir-gateway must not assume either outcome. It should report the write as unconfirmed and leave it to the caller to check the upstream state before trying again, so that we do not create a duplicate appointment.
- The abandoned call should be cancelled cleanly so that connections are not left hanging open and slowly use up the pool. If the HTTP client keeps the socket after a timeout, it should be closed rather than returned to the pool.
- Each timeout is logged with enough context to find it later: which upstream, which kind of operation, and how long it actually ran. The log line must not contain patient names, identifiers or any other clinical content. Keep the log free of protected health information.
- Timeouts are counted as a metric, separately from other upstream failures, so that a rise is visible quickly.

What the front desk sees is a short plain message saying that the outside system did not respond and that the screen is showing what was last known, if anything. The message should say when that data was fetched. It should not show a stack trace or a raw error string from the HTTP client.

## Retries, Sidekiq and the shorter budget

Dropping the limit from the old value makes timeouts more likely on slow but healthy upstreams. That is accepted on purpose. The consequence is that retry behavior matters more, and it needs to be thought about rather than left to defaults.

Interactive requests, the ones where someone is waiting at the desk, should not retry a timed-out call inside the same request. Retrying would double the wait for the person and defeat the reason for the short limit. The better path is to fail fast, show the message, and let the person trigger a refresh.

Background work is different. Sidekiq jobs that sync availability or push appointments can retry, because nobody is waiting on them. For those jobs:

- A timeout counts as a retryable failure, with backoff so that a struggling upstream is not hit again right away.
- Reads are safe to retry. Writes are only safe to retry if they are idempotent on the upstream side or if the job first checks whether the earlier attempt landed. A job that creates an appointment must look for the appointment before creating it again.
- Retries should not pile up. If an upstream is down, many jobs will time out at once and then all come back together. Jitter in the backoff and a cap on how many of these jobs run at the same time keep that from turning into a second outage.
- After the retry budget is used up, the job goes to the usual dead set and an alert fires. It should not retry forever, and it should not silently vanish.

A circuit breaker per upstream is a natural companion to the shorter limit: after several timeouts in a row, stop calling that upstream for a short while and fail immediately. This spec does not require one. It is listed under open points. If one is added later, its trips should be visible in the same place as the timeout metric.

On Heroku, dynos can be restarted at any time, so a job that was waiting on an upstream when the dyno went away will be run again by Sidekiq. This is another reason writes must be safe to repeat.

## Rollout, checks and open points

Order of work, roughly:

- Find where the old 8 second value is set. It may be in the gateway configuration, in an initializer, in an environment variable on Heroku, or hard-coded in a client wrapper. All of these need to end up agreeing on `5 seconds`. A search for the old value and for the words timeout and read timeout across the Rails app and the Heroku config is the way to find them.
- Make the value come from one place. Remove local overrides unless there is a written reason to keep one, and if there is, record it in this note.
- Make sure the connect, send and receive phases do not each get the full budget separately in a way that adds up to much more than the limit. If the HTTP library only supports per-phase limits, set them so the total stays within `5 seconds`, or wrap the call in an overall deadline.
- Update any tests, fixtures and stubs that assumed the old value.
- Update documentation and runbooks that quote the old value, including anything on the clinic-facing side that tells staff how long to wait.

Checks before calling it finished:

- A test with a fake upstream that never answers shows the call being abandoned at the limit and a timeout failure being raised, not an empty result.
- A test with a fake upstream that answers just inside the limit shows the call succeeding.
- A test for a write that times out shows the result reported as unconfirmed and no automatic repeat in the interactive path.
- A test for a background job shows the retry with backoff and the check for an earlier attempt before creating again.
- A manual pass in a staging environment with a deliberately slow upstream, looking at what the front desk screen shows, that the message is readable and that no clinical data ends up in the logs.
- After release, watch the timeout metric for a while. A jump right after the change is expected to some degree, but if a particular upstream times out constantly, that is a sign to talk to that upstream's owner, not a sign to bring the old value back.

Open points that are not settled:

- Whether a circuit breaker per upstream is wanted, and with what thresholds.
- Whether some upstreams that are known to be slow, such as large searches, ever deserve a different limit. For now the answer is no: one value, `5 seconds`, for all. If a case turns up, write it down here with the reason before changing anything.
- Whether the front desk message should offer a retry button or only tell people to refresh.
- Whether stale data may be shown for availability, and for how long. This needs input from the people who run the clinics, because showing old availability has its own risk.

If this note and the code disagree, the code wins on what is true today, and this note wins on what was intended. Fix whichever one is wrong and say so here.
