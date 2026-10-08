---
id: 01KY5C806YWGEE4DPQQ88NTT54
created: 2026-07-22T13:58-03:00
sources:
  - "code: Procfile"
---

# reminder-worker reference

reminder-worker is the Sidekiq process that sends appointment reminders for ClinicSlotter. It runs apart from the Rails web dynos, reads one dedicated queue, and does nothing else. This note covers how it starts, what it depends on, and what to check when reminders go missing. It is written from working knowledge, not from a fresh read of the code, so check details against the repo before relying on them.

## What it is

reminder-worker is a Sidekiq process in the same Rails codebase as the web app. It shares the models, the MySQL database and the initializers. The only difference is the command that starts it and the queue it listens to.

Front-desk staff never see it directly. They see its effects: a patient gets a message before an appointment, and the appointment record shows that a reminder went out.

## How it starts on Heroku

reminder-worker is started on Heroku through a Procfile entry. The command in that entry is:

```
bundle exec sidekiq -q reminders -c 3
```

Heroku runs this on its own dyno type, separate from web. Changing the command means editing the Procfile and deploying. The dyno does not read a separate config file for the queue or the concurrency, because both are on the command line.

## The command, piece by piece

- `bundle exec` makes sure the gems come from the app's Gemfile.lock and not from whatever else is installed on the dyno.
- `sidekiq` is the Sidekiq server process.
- `-q reminders` limits the process to the reminders queue. It will not pick up jobs from any other queue.
- `-c 3` sets the concurrency, meaning how many jobs run at the same time inside this one process.

If someone adds a second queue to the command, the worker will start taking on other work. That is almost never what we want here.

## The queue it reads

Only the reminders queue. Jobs for other things, such as FHIR sync or schedule recalculation, go to other queues and are handled by other workers. Keeping reminders alone means a long FHIR sync cannot hold up a reminder, and a burst of reminders cannot starve scheduling work.

If a job lands in the wrong queue, reminder-worker will never see it. It will sit there until some other process reads that queue. Check the queue name on the job class first when something seems to vanish.

## Concurrency

The concurrency is set by `-c 3` on the command line. It is low on purpose. The database connection pool has to be at least as large as the Sidekiq concurrency, and the dyno is small. Raising the concurrency without raising the pool gives connection timeout errors that look like random job failures.

If reminders are slow to drain, scale the number of dynos before raising concurrency. More dynos is the safer lever.

## What it does for a reminder

In rough order, a reminder job does this:

- Loads the appointment and checks it still exists and has not been cancelled.
- Checks that the appointment time is still in the future and that the slot has not moved since the job was queued.
- Builds the message from the clinic's template, with patient name, clinician, time and location.
- Sends it through the configured messaging provider.
- Records on the appointment that a reminder was sent, so a retry does not send it again.

The job should always reload from the database and never trust values passed in when it was enqueued. Appointments get moved a lot at small clinics.

## Inputs it depends on

- The appointments table and the clinician and room data behind it.
- Per-clinic settings: whether reminders are on, how far ahead they go, and the template.
- Patient contact details and the patient's consent for being contacted.
- Credentials for the messaging provider, held in Heroku config vars and not in the repo.

If any of these is missing, the job should skip and log a reason, not raise. A raise means a retry, and a retry of a bad record is just noise.

## FHIR side

Appointment and patient data can arrive through HL7 FHIR from the clinic's other systems. reminder-worker does not talk FHIR itself. It works from the local records that the FHIR sync wrote. So a stale reminder is often a stale sync, not a worker bug.

When an appointment is changed upstream and the sync has not run yet, the worker can send a reminder for the old time. The reload-before-send step narrows that gap but cannot close it.

## MySQL access and connection pool

Every job opens a connection from the ActiveRecord pool and returns it when done. Keep jobs short and do not hold a connection while waiting on the messaging provider if it can be avoided. Slow provider calls with the pool held are the usual reason a small worker falls behind.

Watch for the database's connection limit across all dynos together. Web, this worker and any other workers draw from the same server.

## Retries and failures

Sidekiq retries failed jobs with growing delays. For reminders that is only partly useful: a reminder that arrives after the appointment is worse than none. The job should check the appointment time at the start of each attempt and drop itself when it is too late to be useful.

### Failures worth retrying

Provider timeouts, rate limits, and short database outages. These are temporary and a later attempt will likely work.

### Failures not worth retrying

Missing appointment, cancelled appointment, no consent, no valid contact detail, or a template that fails to render. Log the reason and finish the job cleanly.

### Dead jobs

Jobs that run out of retries go to the dead set. Look at it after any provider incident. Do not blindly replay dead reminders, since many will be for appointments that have already passed.

## Idempotency

A job can run twice: Sidekiq gives at-least-once delivery, and a dyno restart in the middle of a job will cause a rerun. The sent marker on the appointment is what prevents a double message. Set it as close to the actual send as possible, and check it first thing. There is still a small window between send and marker; we accept that rather than risk missing a reminder.

## Timing and time zones

Reminders are scheduled relative to the appointment's local time at the clinic, not the server's time. Clinics can be in different zones. Store and compare times in UTC internally and convert only when building the message text. Daylight saving changes are a known place for off-by-an-hour mistakes, so test around them.

## Deploys and restarts

Heroku restarts dynos on every deploy and on its regular cycle. Sidekiq is told to stop and gets a short time to finish running jobs. Jobs that do not finish are pushed back to the queue and run again, which is another reason the jobs must be safe to repeat.

After changing the Procfile, confirm in the dyno list that the worker dyno is running the new command, and that it is not stuck in a crash loop.

## Scaling on Heroku

The worker dyno count is set in Heroku, not in the repo. Normally one dyno is enough. Add more when the reminders queue stays deep for a long time, for instance after a bulk import of appointments. Remember each added dyno adds database connections. Scale back down afterwards so the connection budget is not wasted.

## Debugging checklist

- Is the worker dyno up? Check the dyno list and the recent logs.
- Is the reminders queue growing, or empty? Empty with no reminders sent points at enqueueing, not at the worker.
- Did the job go to the right queue?
- Does the appointment still exist, in the future, not cancelled?
- Is reminders switched on for that clinic, and does the patient have consent and a contact detail?
- Is the provider up, and are the credentials in config vars still valid?
- Is the dead set filling up? Read the error on a few entries.
- Did a FHIR sync lag cause the old time to be used?

## Things that look wrong but are not

A low concurrency is intended. An empty queue most of the day is normal, because reminders go out in waves. A job that logs a skip and finishes is working as designed. Retries that stop early on a past appointment are also by design.

## Open questions

- Whether the per-clinic send window should be enforced in the worker or at enqueue time.
- Whether dead reminders should be auto-discarded after the appointment time passes.
- Whether a second queue for urgent same-day reminders is worth the extra process.
