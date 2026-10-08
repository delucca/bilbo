---
id: 01KSF43WHX4JERADMRRPW0YJEM
created: 2026-05-25T05:30-03:00
---

# reminder-worker must send revised reminders

Loose notes on what reminder-worker has to do when an appointment changes after a reminder has already gone out, or is already queued. I'm writing this from memory of the discussion, not from the code, so treat the details as soft and check them against the app before relying on them. The short version: if a slot moves, the patient must get a revised reminder that shows the new time. They must not get a second reminder that looks like the first one, and they must never be told the old time again after the change.

The front-desk staff at these small clinics move appointments all the time. A clinician calls in sick, a room is double-booked and the scheduler shifts someone, or the patient phones and asks for a different day. Each of those produces a changed appointment. If reminder-worker only knows how to send the original reminder, the patient turns up at the old time, and the front desk takes the blame. That is the whole reason this spec exists.

## What the worker is responsible for

reminder-worker is a Sidekiq worker in the Rails app. It reads appointments from MySQL, decides who needs a reminder, and sends it. It does not decide the schedule. The scheduling code owns clinician availability and room constraints, and reminder-worker only reacts to the result. I want to keep that boundary, because every time reminder logic starts to second-guess the scheduler we get two sources of truth for when an appointment happens.

There are really three jobs here, and they are easy to blur together:

- Send the first reminder for an appointment at the usual lead time before it starts.
- Send a revised reminder when the appointment time, clinician, or location changes after a reminder has gone out or been scheduled.
- Send nothing, and say so in the logs, when the appointment was cancelled or has already passed.

The second job is the one this note is about. The first and third already behave roughly as expected. The revised path is where we keep finding holes.

What counts as a change worth a revised reminder is the part I would pin down first. Time changes obviously count. A change of clinician counts if the patient was told the clinician's name in the original message. A change of room counts only if the room is shown to the patient, and from what I remember it mostly is not, so a pure room swap should usually not trigger anything. Notes edited by staff, internal flags, and insurance fields should never trigger a reminder. I would keep this as a short explicit list of fields in one place rather than diffing the whole record, because diffing the whole record means every internal edit wakes the worker up and some patient gets a pointless message.

## When an appointment changes

The trigger should come from the place where the appointment is saved, not from a periodic sweep that tries to notice differences. A sweep is simpler to write but it has a gap: if the appointment moves twice between sweeps, we only see the final state and cannot tell whether a reminder for the middle state went out. Worse, if it moves and then moves back, a sweep sees nothing and the patient is left with a revised reminder that was already sent for a state that no longer exists.

So the plan, loosely:

- When the scheduler commits a change to a relevant field, it enqueues a job for reminder-worker with the appointment identifier and nothing else.
- The worker loads the appointment fresh from MySQL when it runs. It never trusts a snapshot passed in the job arguments, because by the time Sidekiq picks the job up the data may have changed again.
- The worker compares what the appointment looks like now with what the last reminder for it said. That record of what was last sent is the important piece, and it has to be stored, not inferred.

The enqueue should happen after the database transaction commits. This is a known Rails trap: if the job is enqueued inside the transaction, Sidekiq can run it before the commit is visible, load the old row, decide nothing changed, and exit. The patient never hears about the move. Using an after-commit hook for the enqueue avoids this. If anyone sees a revised reminder that mysteriously never went out, check this first.

### Reminders that are queued but not yet sent

A different case is when the original reminder has not gone out yet. The appointment was booked, a reminder job is sitting in the schedule for later, and then the time changes. Here we do not want to send an original and then a revised one back to back. Two options came up:

- Cancel or invalidate the pending job and enqueue a new one for the new time.
- Leave the pending job alone and have it re-read the appointment when it runs, so it naturally sends the right time.

I prefer the second. Deleting scheduled jobs out of Sidekiq's schedule by hand is fiddly and easy to get wrong, and the job should be re-reading the row anyway. The catch is that the pending job is timed for the old appointment, so if the appointment moved later it would fire too early, and if it moved earlier it would fire too late or after the visit. So the job needs to check whether its own intended send time still matches the appointment, and if not, do nothing and let the freshly enqueued job for the new time handle it. That means each reminder job carries the send time it was created for, and compares it with what the appointment implies now.

This is slightly more logic than "just cancel it," but it fails safe. A stale job exits quietly. A missed cancellation would send wrong information.

## Not sending duplicates

Sidekiq delivers jobs at least once. That is a documented property, not a rare edge case, and on Heroku dynos get restarted regularly, which makes retries of half-finished jobs a normal event. The worker has to be safe to run twice for the same change.

The way to do that is a record per reminder that is written as part of deciding to send, with a uniqueness rule so the second attempt cannot create the same record again. Something like: one row per appointment, per kind of reminder (original or revised), per version of the appointment's relevant fields. Before sending, the worker tries to claim that row. If the claim fails because the row exists and is marked sent, it stops. If the row exists but is not marked sent, a previous attempt died partway, and it has to decide whether to retry the send.

That last case is the awkward one. If the send to the messaging provider succeeded but the process died before the row was marked, a retry sends the patient a second copy. If we mark the row first and then send, a crash in between means the patient gets nothing. Neither is great. For appointment reminders I think a rare duplicate is better than a silent miss, so the order should be: claim, send, mark. And the message text for a revised reminder should be written so that a duplicate is harmless, meaning it states the current time plainly and does not say something like "your appointment has just been changed" in a way that would confuse a patient who reads it twice.

There is also the burst case. A front-desk person drags an appointment around a calendar and saves a few times in quick succession. Each save enqueues a job. We do not want a revised reminder for every intermediate state. The usual fix is a short settle delay: the job is scheduled slightly in the future, and when it runs it only proceeds if it is the latest job for that appointment. Any earlier job sees that a newer version exists and exits. The settle delay should be short enough that it does not hurt an appointment that is close, and long enough to absorb a normal flurry of edits. I don't remember what value was floated and I would not copy it from memory; make it a configured setting and look at how staff actually use the calendar before deciding.

## What the revised message says

The revised reminder needs to be clearly a revision. If it reads like the original with different numbers, a patient who still has the first message in their phone will see two reminders that disagree and not know which to believe. So the wording should say that the time has changed, give the new time, and ideally mention the old one once so the patient can match it to what they remember. Keeping the old time in the message is a small thing but it removes most of the confusion that front-desk staff end up fielding by phone.

A few content rules I would hold to:

- Always show the date and time in the clinic's local time zone, not the server's. Heroku runs in UTC, and any place where a time is formatted without explicitly converting to the clinic's zone is a bug waiting to show up as an off-by-hours message. This has to be tested across a daylight saving change, because that is where a naive conversion goes wrong.
- Show the clinic name and the usual contact method for changing or cancelling. Small clinics often have a single phone line, and the message should point there.
- Do not include clinical detail. The reminder goes to a phone, possibly shared, and should contain no more than the minimum: who, where, when. No reason for the visit, no clinician specialty if it hints at a condition.
- If the clinician changed and the original message named the clinician, name the new one. If the original did not name anyone, do not start naming people in the revision.

The patient's contact preferences apply to revised reminders exactly as to the original ones. If someone opted out of reminders, a revision is also a reminder and must not be sent. I mention it because it is tempting to treat a revision as an important service message that overrides preferences. It should not, unless the clinic explicitly decides that and the consent wording supports it. Raise that with whoever owns the clinic agreements before anything is built on that assumption.

## Timing, quiet hours, and appointments that are close

A revised reminder has a send-time problem the original does not. The original is scheduled at a comfortable lead time. A revision is triggered by an edit that may happen at any hour, including late at night if staff are working through a backlog, and possibly very close to the appointment itself.

Three situations:

1. The change happens well ahead of the appointment and during normal hours. Send the revision after the settle delay. Nothing special.
2. The change happens during the clinic's quiet hours. The revision should wait until quiet hours end, unless the appointment is so soon that waiting would mean the patient never sees it. The quiet-hours window is configured, and the logic needs a defined rule for what wins when the two collide. My view is that a revision for an imminent appointment wins over quiet hours, because a patient going to the wrong time is worse than an early message. But this is a judgment call and should be agreed with the clinics, not decided silently in code.
3. The change happens very close to the appointment, or after the point at which we would normally send anything. Then a message may be useless or even misleading, and the right move is probably to flag it for the front desk to phone the patient instead. I do not think reminder-worker should try to be clever here. It should detect that case, skip the automated send, and surface it somewhere staff will see it, such as a task list or a flagged row on the day's schedule.

There is a related subtlety about what "close" means when an appointment is moved to an earlier time. If a patient was booked for the afternoon and is moved to the morning with only a short notice, the revised reminder arrives when they might already have made other plans. Nothing technical fixes that, but it is another reason to have the front desk phone for short-notice moves and treat the automated message as a backup.

Cancellations deserve a line. A cancelled appointment should produce no reminder and should invalidate any pending one. Whether to send a cancellation notice is a separate question and is not part of this spec. I would keep it out of reminder-worker unless someone asks for it, since it has its own wording and consent concerns.

## FHIR and the outside world

Some clinics exchange appointment data with other systems through HL7 FHIR. When an appointment arrives or is updated through that interface, it can change without anyone touching the calendar in our UI. Those changes must go through the same path as manual edits, meaning the same save, the same after-commit hook, and the same field list, so a revised reminder is triggered no matter where the change came from. If the FHIR import writes rows in a way that skips the model callbacks, such as a bulk update, the revised reminder would never be enqueued. That is a likely hiding place for a bug and worth a test that pushes an update through the import path and checks that a job appears.

In the other direction, if we ever publish appointment updates outward, the revised reminder should not be treated as a source of truth for anything. It is a notification derived from the appointment. The FHIR appointment resource carries the status and the start time; reminder-worker reads from our own tables, not from the outgoing payload.

A small mapping point: FHIR has statuses that mean different things for us, such as booked, cancelled, and no-show. Only the booked state should ever get a reminder. Any other state, including ones we do not currently handle, should be treated as "do not send" by default, not "send anyway." Unknown should fail closed.

Here is the rough shape of the flow, only to make the pieces clear:

```
scheduler save (relevant field changed)
  -> after commit: enqueue reminder-worker job (appointment id only)
  -> Sidekiq runs job after settle delay
  -> worker reloads appointment from MySQL
  -> compare with last reminder record
  -> claim record, send revised reminder, mark sent
```

## Running it on Sidekiq and Heroku

Operational points that came up, none of which need exact numbers here.

Retries. Sidekiq retries failed jobs with backoff. For reminder-worker the retry window should be bounded by the appointment itself: once the appointment time has passed there is no value in retrying, and a late revised reminder sent after the visit would be actively wrong. So the job should check on every attempt whether the appointment is still in the future and still booked, and give up cleanly if not. Letting it retry for the default period and send a stale message would be a real problem. It is better to have a dedicated, bounded retry setting for this worker than to inherit the global default.

Queues. Reminders are time sensitive, and they should not sit behind long-running jobs like report generation or bulk imports. A separate queue for reminder-worker with its own concurrency makes sense. I would not give it a very high priority weight over everything else, only enough that a big import cannot starve it. When something does go wrong, the symptom is usually a pile of delayed reminders, so queue latency for this queue is the thing to alert on.

Dyno restarts. Heroku restarts dynos at least daily, and deploys restart them too. Sidekiq is expected to finish or requeue in-flight jobs on a graceful shutdown, but a job that overruns the shutdown grace period is killed and later retried. That is the case the claim-send-mark ordering above is designed for. Worth testing by killing the process mid-send in a staging environment to see what a patient would actually receive.

Database load. The worker reads and writes MySQL on every run. During a morning when staff reshuffle a whole day, because a clinician is out, there will be a wave of revised-reminder jobs for many patients at once. They should be independent and cheap, but the unique claim on the record table is a potential contention point if it is badly indexed. Check that the lookup used for the claim is covered by an index before this goes to a larger clinic.

Provider limits. The messaging provider has rate limits. A day-reshuffle wave can hit them, and a rejected send should be retried within the bounded window, not dropped. The configured limit for outgoing messages belongs in settings, and the worker should back off rather than hammer the provider.

Logging. Every decision should leave a line: sent, skipped because unchanged, skipped because stale, skipped because cancelled, skipped because opted out, deferred for quiet hours, flagged for phone call. Without these, when a clinic says a patient never got a revised reminder we have no way to tell which of a dozen reasons applied. Include the appointment identifier and the reminder record identifier in each line and keep patient names and contact details out of logs.

## Testing and open questions

Tests I would want before calling this done:

- Move an appointment before its original reminder is sent, and confirm exactly one message goes out with the new time.
- Move an appointment after the original went out, and confirm exactly one revised message goes out.
- Move it, then move it back inside the settle delay, and confirm nothing goes out or the result matches the original state.
- Run the same job twice and confirm one message.
- Kill the worker between send and mark and look at what happens on retry.
- Change only a field outside the list, such as internal notes, and confirm nothing is sent.
- Update through the FHIR import and confirm a job is enqueued.
- Change the time across a daylight saving boundary and check the displayed time in the clinic's zone.
- Opted-out patient, cancelled appointment, appointment already in the past: all send nothing.

Open questions I do not have answers to:

- Which fields exactly trigger a revision, and does a clinician change always count? Needs the clinics' view, not just ours.
- What is the rule when quiet hours and an imminent appointment collide, and who signs off on it?
- Where do the "phone the patient instead" cases show up for the front desk, and is there an existing place or does this need a new screen?
- Should a cancellation notice live in reminder-worker or elsewhere?
- How long should the settle delay be, based on how staff really edit the calendar?
- Does the existing reminder record store enough about what was sent to compare against, or does it need extra columns for the previous time and clinician?

There is already another note on this same requirement, I think under a similar name, and I did not check it before writing this one. Where the two disagree, trust whichever matches the code, and merge the two when someone has a moment. The main thing both should keep: reminder-worker must send a revised reminder when an appointment changes, send it once, send it with the right local time, and send nothing when the change makes a message pointless or wrong.
