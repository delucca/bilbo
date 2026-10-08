---
id: 01K4D8Z7SQPKXC0NCMSWJEGS5S
created: 2025-09-05T12:47-03:00
---

# reminder-worker design

reminder-worker is the part of ClinicSlotter that tells patients about upcoming appointments. It is a Sidekiq worker, the class `ReminderWorker`, and it runs on the queue reminders. It is configured with `sidekiq_options retry: 6`. This note records how it is meant to work, why it is shaped that way, and what to check before changing it. It is written quickly from what the team has settled, so it favors concrete statements over polish.

The short version: the Rails app decides when an appointment needs a reminder and enqueues a job. `ReminderWorker` picks the job up from the reminders queue, loads the appointment fresh from MySQL, decides whether a reminder is still appropriate, sends it, and records that it did. If something fails, Sidekiq retries according to `sidekiq_options retry: 6`. After the retries are used up, the job lands in the dead set and someone has to look at it.

## What reminder-worker is and what it is not

reminder-worker does one thing: deliver a single reminder for a single appointment. It does not schedule appointments, it does not check clinician availability, and it does not check room constraints. Those belong to the slotting code in the Rails app. By the time reminder-worker runs, the appointment already exists, and the only question is whether the patient should be told about it now.

The worker is deliberately thin. It takes an identifier for the appointment and nothing else that could go stale. It does not carry the time, the clinician name, the room, or the patient contact details in the job arguments. Front-desk staff move appointments around all day, and a job that was enqueued a long time ago would otherwise send out old information. The job argument is the appointment identifier only. Everything else is read when the job runs.

This matters because ClinicSlotter is used by front-desk staff at small clinics. A small clinic changes its mind often. A clinician calls in sick, a room is taken for something else, a patient phones to move to the afternoon. Each of those edits can happen after the reminder was enqueued. If the worker trusted its arguments, patients would get wrong reminders, which is worse than no reminder. So the rule is: arguments identify, the database describes.

Things reminder-worker is not responsible for:

- Deciding the reminder lead time. That is a clinic setting read when the job is enqueued, and again when the job runs, so a changed setting is respected.
- Rendering the clinic's layout or branding. The worker asks a small message-building object for the text and hands it to the delivery channel.
- Long-term storage of delivery history beyond a simple record that a reminder went out or was skipped.
- Retrying forever. The retry budget is finite on purpose, see below.

## Job lifecycle

The lifecycle has five steps, and each one has a reason.

First, enqueue. When an appointment is created or moved, the Rails side computes when the reminder is due and schedules a job for that time on the reminders queue. If the appointment moves, the old scheduled job is not hunted down and removed. Instead the old job is allowed to run and will decide to do nothing, as described in the next step. This was chosen because finding and deleting a specific scheduled job in Sidekiq is clumsy and racy, while a check at run time is simple and always correct.

Second, load and re-check. When `ReminderWorker` runs, it loads the appointment by identifier. If the appointment no longer exists, it returns without error. If the appointment was cancelled, it returns without error. If the appointment has moved so that the reminder is not yet due, or the reminder for the new time has its own job, it returns without error. Returning quietly in these cases is correct behavior, not a failure, and must not raise. Raising would burn retries and eventually fill the dead set with noise.

Third, check for a prior send. The worker looks at the record of reminders already sent for that appointment and that appointment time. If one exists, it returns. This is what makes the job idempotent, which is the single most important property of the worker given that retries exist. A job that is retried after a partial failure must not send the patient a second copy.

Fourth, send. The worker builds the message, picks the channel the patient has agreed to, and hands it to the delivery client. Delivery is the only step that talks to the outside world, and so it is the step most likely to fail.

Fifth, record. After a successful hand-off, the worker writes the sent record. There is a small window between a successful hand-off and writing the record. If the process dies inside that window, the retry will send again. We accept that. A duplicate reminder is a minor annoyance; a missed reminder means an empty slot in a small clinic's day, which costs real money. The design leans toward sending twice rather than zero times.

## Retry policy

The worker is declared with `sidekiq_options retry: 6`. That means a failed job is retried up to six times, with Sidekiq's built-in exponential backoff between attempts, before it is moved to the dead set. The value is explicit in the class rather than inherited from the Sidekiq default so that a reader of the worker sees the intent and so that a change to global defaults does not silently change reminder behavior.

Why a finite number and why this one. Reminders are time-sensitive. A reminder that arrives after the appointment has passed is useless, and one that arrives very late the day before is nearly useless. The backoff curve with a handful of retries spreads attempts over roughly a day, which is the useful window for most reminders. More retries would push attempts well past the point where the reminder has any value, and would keep failing jobs alive long enough to confuse people looking at the queue. Fewer would give up too early on a short outage of the delivery provider or the database.

Because of the re-check in the lifecycle, a late retry is safe. If a retry runs after the appointment time has passed or the appointment was cancelled in the meantime, the worker sees that and returns without sending. So the retry count is bounded by usefulness, not by safety.

What counts as a retryable failure:

- Timeouts and connection errors talking to the delivery provider.
- Temporary database errors, such as a lost connection or a lock wait timeout in MySQL.
- Rate limiting responses from the provider.

What should not be retried, and should be handled inside the worker without raising:

- A patient with no valid contact method. Retrying does not create a phone number. The worker records a skipped reminder with a reason and returns.
- A patient who has opted out of reminders. Same handling.
- A permanent rejection from the provider, such as an address that is known to be invalid. Record it, mark the contact as suspect, and return.

The rule of thumb: raise only when trying again could plausibly change the outcome. Everything else is a recorded decision, not an error. If you add a new failure path, decide which of the two it is before you write the rescue.

When the retries are used up, Sidekiq moves the job to the dead set. We do not currently have automatic resurrection from the dead set. A person looks at it, decides whether the reminder still matters, and either re-enqueues manually or lets it go. Most of the time, if it is dead, the appointment time has probably passed or is close enough that a front-desk call is the better channel anyway.

## Data, FHIR, and the database

ClinicSlotter keeps its appointment data in MySQL, and exposes and consumes appointment information over HL7 FHIR for clinics that connect other systems. reminder-worker reads from the local MySQL tables, not from a FHIR endpoint. This is intentional. Reading from the local store is fast, is consistent with what front-desk staff see on screen, and does not make reminder delivery depend on a remote system being up.

There is one caveat. When an appointment arrives or is changed through the FHIR interface, it is written to the local tables first and then the same enqueue logic runs as for an appointment created by hand. So from the worker's point of view, there is only one kind of appointment. If you ever find yourself making the worker aware of where an appointment came from, stop and ask why; the answer is usually that the enqueue side is missing something.

Time zones deserve a warning. Each clinic has its own local time zone, and a reminder is expressed in the clinic's local time. The worker must convert for display only at the last step, and compare times in a single consistent zone before that. Past bugs in this area were all the same shape: a comparison done on a local time against a stored time in another zone, which made a reminder appear due too early or too late around daylight saving changes. When touching the due-time logic, test with a clinic near a daylight saving boundary.

The sent record is the other piece of state. It is keyed by the appointment and the appointment time it was sent for, not just the appointment. That way, when an appointment is moved, the new time is eligible for its own reminder, while a retry for the same time is still recognized as a duplicate. If you change the key, you change the idempotency behavior, so treat that table with the same care as the job itself.

A few points on database load. The worker does a small number of indexed reads per job and one write. Reminders tend to cluster, since many clinics book at the top of the hour and many reminders fall due at the same moment each morning. That cluster is the main load on the reminders queue. It has not been a problem on the current database size, but it is the first thing to look at if reminder latency grows.

## Operating it on Heroku

The app runs on Heroku, and Sidekiq runs as its own process type, separate from the web dynos. The reminders queue is one of the queues that worker process listens to. Because it shares a process with other queues, a flood of slow jobs elsewhere can delay reminders. We handle that through queue ordering and weights in the Sidekiq configuration rather than by running a dedicated process, which keeps the bill down for a product aimed at small clinics. If reminder delay ever becomes a complaint, a dedicated process for the reminders queue is the obvious next step, and nothing in the worker code would need to change.

Things to know when something looks wrong:

- Reminders not going out at all. Check that the Sidekiq process is running and that the reminders queue is being listened to. Then check the scheduled set, to see if jobs are waiting for their time. Then check whether enqueue is happening, by looking at whether recently created appointments have a scheduled job.
- Reminders going out twice. Look at the sent record. If it exists for that appointment and time, the second send came from somewhere other than the worker's normal path, such as a manual re-enqueue or a second code path that sends messages. If it does not exist, the write after delivery is failing, and each retry is sending again. That is the most likely real cause.
- Reminders with old details. This should not be possible given that arguments carry only the identifier. If it happens, someone has added data to the arguments, or a cache is sitting between the worker and the database. Remove it.
- The dead set growing. Look at the error classes. A burst from one provider error means the provider had an outage that outlasted the retries. A steady trickle usually means a permanent failure that is being raised when it should have been recorded and returned.

Deploys restart the Sidekiq process. Sidekiq gives running jobs a chance to finish and puts unfinished ones back on the queue, so a deploy during a busy morning is safe, but only because the worker is idempotent. That is one more reason to protect the idempotency check.

Logging should include the appointment identifier and the outcome of each run: sent, skipped with a reason, or failed. Do not log patient names, contact details, or message text. These are clinics handling health information, and logs on a hosted platform are not the place for it. The outcome and the identifier are enough to trace a problem back through the database.

## Decisions, tradeoffs, and things to revisit

Decisions that are settled:

- The job argument is the appointment identifier only, and the worker re-reads everything.
- Stale jobs are tolerated and neutralized at run time instead of being cancelled when an appointment moves.
- The worker is idempotent through a sent record keyed by appointment and appointment time.
- Retries are finite, set explicitly with `sidekiq_options retry: 6`, and permanent problems are recorded rather than raised.
- Reading is from local MySQL, never from FHIR, at delivery time.
- Prefer a rare duplicate over a missed reminder.

Tradeoffs we accepted. Running stale jobs means the scheduled set holds more entries than strictly needed, and every moved appointment leaves a harmless no-op job behind. The cost is small and the simplicity is worth it. The duplicate window between hand-off and recording is real but narrow. The shared worker process means reminder timing is only as good as the rest of the queues allow.

Things to revisit if the product grows:

- A dedicated process or dedicated capacity for the reminders queue, if clustering at peak hours starts to delay sends.
- A tool or small admin screen to inspect and re-enqueue dead reminders, so front-desk or support staff do not need console access.
- Per-clinic quiet hours, so reminders are not sent at night in the clinic's local time. This would be a check in the re-check step, which already has the right shape for it.
- Making the delivery client swappable per channel, so adding a channel does not touch the lifecycle.
- A way to tell the difference between a retry that was caused by the provider and one caused by our own database, in metrics, since the response differs.

Checklist before changing reminder-worker:

- Does the change keep the job arguments down to an identifier?
- Does every early exit return quietly rather than raise?
- Does a retry after a partial failure still avoid sending twice, or at least only in the narrow window described above?
- Does a new failure path decide explicitly between retry and record-and-return?
- Does anything new end up in the logs that identifies a patient?
- If due-time logic is touched, was it tried across a daylight saving change for a clinic in a different time zone from the servers?

If all of those hold, the change is very likely safe. If one does not, write down why in this note, because the next person will not remember the reasoning and the code alone will not explain it.
