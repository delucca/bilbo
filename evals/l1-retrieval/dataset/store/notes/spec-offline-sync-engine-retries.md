---
id: 01KQQR1MF2J48WKT298KGVHRPQ
created: 2026-05-03T17:21-03:00
---

# offline-sync-engine spec

This note specifies how offline-sync-engine behaves in ParcelPin. The component moves proof-of-delivery records from the courier's phone to the backend when the network is bad or missing. The headline retry rule: offline-sync-engine retries failed uploads with exponential backoff that starts at 30 seconds and gives up after 6 attempts. Everything else here explains the parts around that rule, so a reader who only has this note can reason about what happens to a delivery record from capture to confirmation. Where a detail is not settled, it is described in general terms on purpose and should be checked against the code before anyone relies on it.

## Purpose and scope

Drivers work in basements, lifts, rural roads and dead zones. They still have to capture a signature, a photo, a location fix and a few notes at the door, and move on to the next stop. offline-sync-engine exists so that capture never waits on the network. The driver finishes the stop, the record is saved on the device, and the engine is responsible for getting it to the backend later.

In scope: the local queue of pending records, the upload of those records, the retry policy, the handling of conflicts, the reporting of sync state to the UI, and the cleanup of local data after confirmed upload. Out of scope: how the capture screens look, how routes are assigned, and how the dispatcher dashboard reads data. Those consume the results of the engine but do not define its behavior.

The engine is a library inside the Android app written in Kotlin. It uses Android Jetpack pieces for background work and local storage, Firebase for authentication and file storage, Cloud Firestore for the structured delivery data, and Protocol Buffers for the shape of the payload that moves between app and backend.

## Design goals

The first goal is that a driver never loses a proof of delivery. A record that was captured must survive app restarts, phone reboots, a dead battery and a long period offline. The second goal is that the driver never has to think about sync. There is no manual upload button in the normal flow, though a status indicator tells the driver when things are stuck. The third goal is that the backend can trust what it receives: each record arrives complete, once, and in a form it can validate.

A fourth goal is to be gentle with the device. Couriers carry phones all day on a single charge and often on metered data. The engine should not hammer the radio, should not retry in a tight loop, and should wait for sensible conditions before sending large photo files. This is the main reason the retry policy uses growing delays instead of a fixed short one.

Non-goals: real-time delivery of every record, strict global ordering of records across drivers, and editing a record after it has been confirmed on the backend. Real-time is nice when the network is good, but the design does not promise it.

## Data flow overview

A delivery record starts as a capture in the app. The capture layer builds a record, writes it to the local store in one transaction together with references to its attachments, and marks it as pending. It then asks the engine to schedule a sync. The engine picks up pending records, prepares each one for upload, sends attachments first and the structured record second, and waits for confirmation from the backend. On confirmation, the record is marked as synced and becomes eligible for cleanup.

If any step fails, the record stays pending and the failure is classified. Transient failures lead to a retry under the backoff rule. Permanent failures move the record to a held state that needs attention. The UI observes the local store, so the driver sees the state change without any extra plumbing.

The important property is that the local store is the source of truth on the device. The network is only ever a side effect of what is in the store. Nothing in the UI reads directly from an in-flight request.

## Local queue

The queue is a table in the local database, managed through Jetpack storage libraries. Each row is one delivery record with a state, a count of how many upload attempts have been made, the time of the last attempt, the time the next attempt is allowed, and a pointer to its attachments on disk. Attachments are kept as files in app-private storage rather than blobs in the database, so large photos do not bloat queries.

States are kept deliberately few: pending, uploading, synced, and held. Pending means waiting for its turn. Uploading means an attempt is in progress right now. Synced means the backend confirmed it. Held means the engine stopped trying and a person or a later app version must act. A record in uploading state after an app restart is treated as pending again, because the process that owned the attempt is gone.

Ordering inside the queue is by capture time, oldest first, with the next-allowed-time acting as a gate. A record whose next attempt is in the future is skipped, and the engine moves on to the next one so a single stubborn record does not block the rest.

## Triggers for a sync run

A sync run starts for several reasons. First, right after a capture is saved, as a best effort. Second, when connectivity returns, signalled by the platform's network callbacks. Third, on a periodic schedule handled by Jetpack background work, so records still get sent if no other trigger fires. Fourth, when the app comes to the foreground. Fifth, when the driver taps retry on a held or stuck record.

Triggers are coalesced. If three of them fire close together, only one run happens. The engine uses unique work semantics so two runs never overlap. This avoids double uploads and keeps the attempt counter honest.

A trigger does not override the backoff gate. If a record is not yet allowed to try again, a network-returned event does not force it. The one exception is the explicit driver retry, which is described under manual retry below.

## Upload sequence for one record

For each eligible record the engine does the following in order. It checks that the user is still signed in with Firebase authentication and refreshes the credentials if needed. It uploads each attachment to cloud storage, skipping any attachment already marked as uploaded from an earlier attempt. It builds the Protocol Buffers payload for the structured part. It writes the record to Cloud Firestore through the backend path. It waits for the acknowledgement and then marks the record synced.

Splitting attachments from the structured write matters. If the structured record landed first and the photo failed, the dispatcher would see a delivery with no proof. By sending attachments first, a record only appears on the backend when its evidence is already there. The cost is that a failure midway leaves orphan files in storage; those are harmless and are reused on the next attempt because the engine remembers which attachments are done.

Each attempt increments the attempt counter once, whether it gets as far as the attachments or fails at the first step.

## Retry and backoff policy

This is the core rule. offline-sync-engine retries failed uploads with exponential backoff that starts at 30 seconds and gives up after 6 attempts. After the first failure the record waits 30 seconds before it may be tried again. Each later failure doubles the wait of the one before. After the sixth failed attempt the engine stops retrying that record automatically and moves it to the held state.

Some practical readings of that rule. The counter belongs to the record, not to the whole queue, so one bad record does not slow the others. The delay is a minimum, not an exact time: the background scheduler may run the work later than the allowed time, and the engine never runs it earlier. The wait is stored as a next-allowed time in the queue row, so it survives restarts; killing the app does not reset the backoff or the count.

Giving up is not the same as deleting. A record that has used all of its attempts keeps its data and its files. It is only removed from automatic retry.

## Jitter and fairness

When many drivers come back online at once, for example after a depot Wi-Fi outage or a regional mobile network recovery, fixed delays would line up and produce a burst against the backend. To soften that, a small random spread is applied to each wait. The spread is small relative to the base delay and does not change the documented rule: the wait never falls below the base value for that attempt.

Inside a single device, fairness comes from the queue ordering. Older records go first, but a record in backoff is skipped rather than blocking. If all remaining records are in backoff, the run ends and the scheduler is asked to wake the engine at the earliest allowed time.

If the exact amount of jitter matters for a change, read the code, since this note intentionally does not pin it down.

## Failure classification

Not every failure deserves a retry. The engine sorts errors into transient and permanent.

Transient: no connection, timeouts, server errors, rate limiting, and expired credentials that can be refreshed. These count as an attempt and follow the backoff rule.

Permanent: the backend rejects the payload as invalid, the record refers to something that no longer exists, the user lacks permission, or a local attachment file is missing or unreadable. A permanent failure moves the record straight to held without using up the remaining attempts, because retrying will give the same answer.

Unknown errors are treated as transient so a record is not parked by mistake, and they still stop at the attempt limit. The classification lives in one place in the code so that new error types are added there and not scattered in call sites. When adding a case, prefer transient if unsure, since the attempt limit bounds the cost.

## Held records

A held record is one that the engine will not touch on its own. It got there either by using up all its attempts or by hitting a permanent failure. The record keeps the reason it was held, in a short code and a human-readable message, so support staff and the driver can see what happened.

The UI shows held records in a clearly separate list with a plain explanation. The driver can try again from there, which resets the attempt count for that record, or can flag it for support. A later app version may also release held records automatically if the cause was a known bug that has been fixed; this is done by a migration step, not by the sync loop.

Held records are never silently discarded, and they are excluded from the cleanup of synced data. A long-lived held record is a signal worth surfacing to the backend as a metric, so operations can spot a systemic problem.

## Manual retry

The driver can force a retry for a held record or for a record that is waiting in backoff. This is the one case where the next-allowed time is ignored. A manual retry on a held record starts a fresh attempt sequence, meaning the count goes back to the beginning and the 30 seconds starting delay applies again if the attempt fails.

A manual retry on a record that is merely waiting in backoff runs one attempt now, and its outcome is added to the existing count rather than resetting it. This keeps the driver from accidentally extending the total number of automatic attempts by tapping repeatedly. If that attempt fails, the normal doubling continues from where it was.

The retry action is rate limited in the UI so a nervous driver does not queue up many parallel runs. The unique work rule on the engine side is the real guard; the UI limit is only for feel.

## Idempotency and duplicate prevention

Because attempts can fail after the backend already accepted the write, such as when the acknowledgement is lost, the engine has to be safe to repeat. Every record gets a stable identifier at capture time, generated on the device and stored with the record. The same identifier is used for the document in Cloud Firestore, so a repeat write overwrites the same document instead of creating a second one.

The Protocol Buffers payload carries this identifier and a version field. The backend uses them to ignore a write that is older than what it already holds. This means a late retry of an old attempt cannot damage newer data.

Attachments follow the same idea: their storage names derive from the record identifier and the attachment role, so a repeated upload replaces rather than duplicates. This is why the engine can skip or redo attachments freely without special cleanup.

## Payload format

The structured part of a delivery record is sent as a Protocol Buffers message. The schema covers the record identifier, the courier and stop references, the capture time, the location fix and its accuracy, the outcome of the delivery, optional notes, and references to the attachments. Attachment bytes are not inside the message; only their storage references are.

Schema changes follow the usual Protocol Buffers rules: add new fields rather than reusing old numbers, never change the meaning of an existing field, and keep unknown fields tolerated on both sides. This matters for offline use in particular, because a phone can sit on an old app version holding queued records for days. The backend must keep accepting old shapes for a long while, and the new app must still be able to read records it queued under an older schema.

The local queue stores the serialized message or the data needed to rebuild it, along with the schema version it was built under. The engine does not rewrite queued records on app upgrade unless a migration says so.

## Conflict handling

Conflicts are rare because a delivery record is written by one courier and normally not edited by anyone else. The cases that do occur are: the same stop being reassigned to another courier while the first was offline, and a dispatcher correcting a record on the backend while the courier still holds an older copy.

The rule is that the backend decides. The engine sends the version it knows. If the backend answers that the stored version is newer or that the stop has changed owner, the engine treats it as a permanent failure for that record and moves it to held with a reason that says so. It does not try to merge. The proof of delivery is evidence, and silently merging evidence is worse than asking a person to look.

For the courier's own device, a conflict never deletes the local copy. The photo and signature stay available so support can still resolve the dispute.

## Network and battery conditions

The engine asks the platform to run uploads when a connection is available, and it separates small and large work. The structured record is tiny and may go over any connection. Photos and other big attachments may be held back on a poor or metered connection, depending on an app setting, until a better one appears. The record still shows as pending so the driver knows it is not finished.

Battery saver and doze modes can delay background work. The engine does not fight them. It relies on the Jetpack scheduler to run when allowed, and on the foreground trigger for the case where the driver is actively using the phone. Because the backoff gate is stored by time and not by timer, long delays caused by power management do not break anything; the record is simply tried when the system lets the work run, provided the allowed time has passed.

A foreground notification is used only for long uploads that need to survive the app being swiped away, and is kept low-key.

## Local cleanup

Once a record is synced, its local copy is not removed instantly. The driver may want to see recent deliveries, and support may ask about them. The engine keeps synced records for a limited period and then deletes the row and its attachment files. The period is a configuration value, not a constant in the sync loop.

Cleanup never touches pending, uploading or held records. It also checks that the attachment files belong to a synced record before deleting them, to avoid removing something still needed for a retry. If storage on the phone runs low, a more aggressive cleanup of old synced data can run, but never of anything unsynced.

Sign-out is a special case. If a driver signs out with unsynced records, the app warns first. The records are kept on the device and are tied to the identity that captured them, so a different user signing in on the same phone cannot upload or see them.

## Observability

The engine reports its own state so problems can be seen without a debugger. On the device, it exposes counts of pending, uploading, and held records and the time of the last successful sync to the UI layer. The driver sees a small indicator, and a detail screen lists records with their state and the reason when held.

On the backend side, the app sends analytics events for sync outcomes: attempt results by error class, time from capture to confirmation, and how many records reach the held state. These events are the main way to notice a regression, such as a new app build that fails against the backend for a subset of devices. Events carry no photos and no personal data about recipients.

Logs on the device are short and avoid recipient names and addresses. Error messages from the backend are mapped to the classification described above before they are stored with the held reason.

## Testing notes

The retry logic is pure enough to be tested without the network. A fake clock and a fake uploader let tests check that the first failure waits the base delay, that each later failure doubles, that the count stops at the limit, and that the record ends in held. These tests should be updated together with the rule if it ever changes, and the figures in this note should be changed in the same commit.

Other tests worth keeping: restart in the middle of an attempt returns the record to pending; a repeated write with the same identifier produces one document; a permanent failure skips the remaining attempts; a missing attachment file leads to held; manual retry on a held record resets the count while manual retry on a waiting record does not.

On a real device, the useful manual checks are airplane mode during capture, switching networks during an upload, killing the app mid-upload, and a long offline period followed by reconnect.

## Open questions

Whether the held state should auto-release after a quiet period, for transient causes that outlast the attempt limit, is undecided. Drivers in long dead-zone stretches could hit the limit just because the schedule of attempts is shorter than the outage. One option is a slower, second-tier schedule after the first one is exhausted; another is to keep the current rule and rely on foreground triggers and manual retry. Nothing here changes the stated rule until that is decided.

Another open point is whether jitter should be documented with exact bounds. For now it is described only in general terms.

A third is how to treat very large attachments on weak connections: chunked, resumable uploads would help, but the current engine restarts a file from the beginning on a failed attempt. If this becomes a visible problem, treat it as a separate change and note it separately from the retry rule.

## Quick reference

Retry rule: exponential backoff, starting at 30 seconds, giving up after 6 attempts, then the record is held.

Transient errors use attempts; permanent errors go straight to held.

The local store is the source of truth; the UI reads the store, not the network.

Attachments go first, the structured record second, with the same record identifier throughout so repeats are safe.

Manual retry on a held record resets the sequence; on a waiting record it adds one attempt to the count.

The backend wins in a conflict, and the engine does not merge.

Held and unsynced data is never cleaned up automatically.
