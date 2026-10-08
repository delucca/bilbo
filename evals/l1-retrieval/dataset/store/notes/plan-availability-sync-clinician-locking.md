---
id: 01KQ8CE068VHBPJHTBQDAKF7Z0
created: 2026-04-27T18:09-03:00
---

# Plan: per-clinician locking in availability-sync

The plan is to add per-clinician locking to `availability-sync` using the MySQL function `GET_LOCK`, so that concurrent jobs for one clinician are serialized. Jobs for different clinicians should keep running in parallel as they do now. In shorthand and in some job and log names this component is called `avsync`; it is the same thing as `availability-sync`. This note is the plan only. Nothing is built yet, and the open questions near the end need an answer before the work is called done.

## Why this plan exists

Front-desk staff at small clinics depend on the slot grid being right. When two `availability-sync` jobs for the same clinician run at the same time, each one reads the clinician's current availability, computes a new set of open slots, and writes it back. The second writer can overwrite or half-overwrite what the first one wrote. The symptom is a slot that shows as open when the clinician is blocked, or a slot that vanishes for no reason. Both are bad at a front desk, where someone is on the phone with a patient while looking at the screen.

The duplicates come from several places. A clinician's calendar can change twice in quick succession, and each change enqueues a sync. A manual resync from the admin side can overlap with a scheduled one. A retry from Sidekiq can land while the original attempt is still running if the original was slow rather than dead. None of these is rare enough to ignore, and none is easy to remove at the source, so the plan is to make overlap harmless instead of trying to prevent it.

## What we want out of it

The goal is narrow. For any one clinician, at most one `avsync` job does its read-compute-write cycle at a time. The rest wait briefly or give up cleanly. It does not need to be fair, and it does not need to preserve the order in which jobs were enqueued, as long as the last job to run sees the latest data. That last point matters more than ordering: a job that runs after waiting for the lock must re-read availability after it gets the lock, not before.

It also should not make anything worse for the clinics that never hit the problem. No extra table, no new service, no new infrastructure. The database is already there and already used by every job, which is the main reason to use `GET_LOCK` and not something external.

## How availability-sync runs today

The work is done in Sidekiq jobs on Heroku worker dynos. A job takes a clinician, pulls the availability inputs for that clinician (working hours, time off, existing bookings, room constraints), and rewrites the derived slots. Some inputs come in over HL7 FHIR from clinic systems that expose schedules; others are entered directly in the app. The job writes through ActiveRecord to MySQL.

There is no coordination between jobs today beyond whatever transaction a single write happens to use. A transaction protects one write from being torn, but it does not protect the whole read-compute-write cycle from interleaving with another one. That gap is what the lock closes.

## Chosen approach: GET_LOCK

MySQL offers named advisory locks through `GET_LOCK`, with a matching release function and a check function. The lock is held by the database session that took it, it is not tied to any table or row, and it is released when the session ends if the code forgets. That last property is the safety net: a crashed worker cannot hold a clinician hostage forever, because its connection goes away.

The shape of the change is a small wrapper around the body of the `availability-sync` job. The wrapper derives a lock name from the clinician, asks for the lock with a timeout, runs the job body if it got it, and releases the lock in an ensure block. If it did not get the lock before the timeout, it takes the failure path described below. Everything else in the job stays as it is.

Other options were considered and set aside:

- A row lock on the clinician record. This ties up a normal table row for the length of a sync, which can block unrelated reads and writes from the app, and it requires holding a transaction open for the whole job. That is worse than an advisory lock.
- A Redis-based lock. Redis is present because of Sidekiq, but a second lock service means a second failure mode, and the lock and the data it protects would live in different systems.
- Sidekiq uniqueness by clinician. This drops or delays enqueues instead of serializing execution, and it does nothing for jobs already running. It could still be added later as an optimization, but it is not a substitute.
- Optimistic versioning on the slot rows. It would detect the conflict after the work is done, which means wasted work and a retry loop. It is a reasonable second layer, not the first.

## Lock naming

The lock name must be unique per clinician and must not collide with any other advisory lock the app uses. The name should include a fixed prefix that identifies `availability-sync`, followed by the clinician's identifier. Two things to remember about MySQL named locks: the name length is limited, so the prefix should stay short, and the names are global to the MySQL server, not to the database schema. On a shared server, a name that does not mention the application could collide with another application's lock. Including the app and component in the prefix avoids that.

If the app ever runs several tenants against one database, the tenant identifier should be part of the name as well. Today the clinician identifier is unique across the database, so the clinician alone is enough, but this should be checked before shipping and not assumed.

## Timeout and what happens on a miss

`GET_LOCK` takes a timeout. Too short and healthy jobs will give up when another job is simply busy; too long and a stuck job ties up Sidekiq threads that other work needs. The timeout should be chosen from how long a normal sync for a typical clinician takes, with some headroom, and should be a setting rather than a literal in the code so it can be changed without a deploy of logic.

When the lock is not obtained, the job should not run its body. The preferred behaviour is to raise a specific, named error that Sidekiq will retry with its normal backoff. The retry will then run later, take the lock, re-read the data and finish the job. That keeps the guarantee that the last job to run sees the latest data. Silently skipping is only right if we know the job that holds the lock started after the data change that enqueued the skipped one, and we cannot know that cheaply. So retry, do not skip.

The return value of `GET_LOCK` has more than two cases. It can report success, a timeout, or an error such as running out of memory or the thread being killed. The wrapper must treat anything other than success as not holding the lock, and must not call the release function in that case, since releasing a lock you do not hold returns a value that means nothing and can mislead the logs.

## Connection and session handling

This is the part most likely to go wrong. A `GET_LOCK` lock belongs to one MySQL connection. ActiveRecord hands out connections from a pool, and a thread may get a different connection between two statements if it checks one in and out. The wrapper must take, use and release the lock on the same connection, which means doing the whole thing inside a single connection checkout for the duration of the job body.

If the job body itself uses other threads or lets the connection go back to the pool partway through, the lock can end up held by a connection that is now serving someone else, or released from a connection that never held it. The job body for `availability-sync` should be checked for this. If anything in it spawns threads or explicitly releases connections, that needs to be fixed or the lock approach needs another design.

Also check how a stale connection is handled. If the connection drops mid-job, the database releases the lock on its own, and the job body keeps running without protection until it notices. Reconnect logic that silently opens a new connection would hide this. The safest behaviour is to treat a lost connection as a failed job and let the retry start from scratch with a fresh lock.

A related point: a lock taken on a connection is not released by a transaction commit or rollback. It survives both. So the release call must be explicit and must sit in an ensure block that runs on every exit path, including exceptions raised from the job body.

## Interaction with Sidekiq

The lock is held while a Sidekiq thread is busy. If many jobs for one clinician pile up, each one waiting for the lock occupies a thread for up to the timeout. A burst of changes for a single popular clinician could therefore soak up a noticeable share of a worker's concurrency, starving jobs for other clinicians and for other queues running on the same process.

Two mitigations, in order of preference. First, keep the timeout short and rely on retries, so waiting threads are freed quickly. Second, if the pile-up still shows up in practice, collapse redundant enqueues, so that only one pending `avsync` job per clinician sits in the queue at a time. That second step is the uniqueness idea from above, used as a complement and not as the main guard.

Retries need a look as well. Sidekiq's retry count is limited, and a job that loses the lock race on every attempt will end up in the dead set. That should be rare once the timeout is sensible, but the dead set should be checked after rollout for `availability-sync` entries that failed on the lock error and not on data problems.

The related note [[reminder-worker-provider-rejects]] covers another worker that deals with retries and external failures. The retry handling there is worth reading before settling how this job reports a lock miss, so the two behave consistently and the on-call person sees familiar patterns.

## Heroku and MySQL considerations

On Heroku, worker dynos restart regularly and can be killed on deploy. A dyno that is stopped mid-job drops its connections, and MySQL releases the locks held by them. That is the behaviour we want, but there may be a delay before the database notices a dead connection if the network disappeared without a clean close. The server-side idle timeout and any TCP keepalive settings decide how long a ghost lock could linger. This needs a check against the actual database add-on's settings, not a guess.

Some hosted MySQL setups sit behind a proxy or connection pooler. A pooler that multiplexes sessions can break advisory locks completely, because consecutive statements from one client may run on different server sessions. Before building anything, confirm that the app's connection to MySQL is a direct session-level connection. If it is not, this plan does not work as written and we need to go back to the alternatives.

Also confirm the MySQL version in use supports holding several named locks per session. Older versions release the previous lock when a new one is requested. We only plan to hold one at a time, so this is not blocking, but the code should never take a second lock inside the first one without knowing which behaviour applies.

## FHIR inputs and the read-after-lock rule

Some of the inputs to `availability-sync` come from FHIR resources fetched from clinic systems. A fetch over the network is the slowest part of a sync and the part most likely to vary in time. Two design choices follow.

First, do the fetch inside the lock, after acquiring it, so the data used is as fresh as possible and a waiting job does not act on a copy that was fetched before a competing job finished. The cost is a longer hold time. Second, if the hold time turns out to be a problem, fetch outside and then re-validate a cheap marker inside the lock, such as a last-modified indicator, and refetch only when it changed. Start with the first, simpler choice and measure.

Whatever is chosen, the rule stays the same: no job may compute from inputs it read before it held the lock.

## Rollout

The change should go out behind a setting that turns the locking on or off without a code deploy. With the setting off, the wrapper just runs the job body, so behaviour is exactly as before. That gives a quick way back if the lock turns out to cause stalls.

Suggested order of work:

- Add the wrapper and the named lock-miss error, with the setting off by default.
- Add logging around acquire, wait, miss and release, tagged with the clinician identifier and the job identifier, so a stuck lock can be traced.
- Test on staging with deliberately overlapping jobs for one clinician and for several clinicians.
- Turn it on in production for the whole fleet at once, since partial enabling would leave some jobs unprotected and make the logs harder to read. If a gradual path is wanted, enable it per clinic, not per job.
- Watch the queue latency and the retry and dead sets for a few days, then remove the setting if nothing odd shows up.

## Testing

Unit tests can stub the lock calls and check that the job body is skipped and the named error is raised on a miss, that release runs after an exception in the body, and that release is not called when the lock was not obtained.

The real behaviour needs a test against an actual MySQL server with two connections, because that is the only way to see that the second connection really waits. A test that uses threads, each with its own checked-out connection, can start one job that holds the lock for a while, start a second job for the same clinician, and assert that the second waits or misses according to the timeout. A third job for a different clinician should proceed at once. Tests that run inside a wrapping transaction with a single shared connection will not show any of this, so those tests should be set up to avoid transactional wrapping.

Also test the failure path where the connection is closed mid-job, to confirm that the lock is gone on the server and that the next attempt gets it.

## Monitoring and debugging

When something looks stuck, there are two places to look. MySQL can report who holds a named lock, through the performance schema metadata lock tables when that instrumentation is enabled, and through the function that checks lock ownership. The first needs the instrumentation turned on, which may not be the case on the hosted database; check that early so it is not a surprise during an incident. The second only answers for one name at a time, so the logs with the clinician identifier are the practical starting point.

Useful signals to add: how long jobs wait for the lock, how often they miss, and how long they hold it. A rising hold time points at slow FHIR fetches or slow writes. A rising miss rate for a single clinician points at an enqueue storm for that clinician.

## Risks and open questions

- Whether the production connection goes through anything that breaks session-level locks. This blocks the whole plan and is the first thing to verify.
- What timeout to use, and whether it should differ for manual resyncs versus scheduled ones. A manual resync from a person waiting on the screen probably deserves a more patient wait.
- Whether any code path in the job body hands the connection back or uses another thread.
- Whether other parts of the app write the same derived slots outside `availability-sync`. If they do, the lock protects only part of the problem, and those writers would need to take the same lock or be routed through the job.
- How the lock error should look to people on call. It is expected and transient, so it should not page anyone on its own; only a high rate of it should.
- Whether to add collapsing of redundant enqueues now or wait for evidence.

## Done when

The plan is finished when the wrapper is live in production, concurrent `avsync` jobs for one clinician are shown in logs to run one after another, jobs for other clinicians are not slowed, and the dead set shows no new `availability-sync` entries caused by the lock. After that, the setting can be removed and this note can be replaced by a short decision note recording the final timeout and the lock name scheme.
