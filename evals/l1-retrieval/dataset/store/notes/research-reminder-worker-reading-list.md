---
id: 01KKSPSZRWHAJ59NREGYKPDV1R
created: 2026-03-15T18:35-03:00
---

# reminder-worker: what we read

These are reading notes on reminder-worker, the Sidekiq-based piece of ClinicSlotter that sends appointment reminders to patients. Nothing here is a decision. It is what the team picked up from docs, old tickets and the code, kept so the next person does not start from zero.

## What it does

reminder-worker takes appointments that are booked and produces outbound messages ahead of the visit. It runs as background jobs on Sidekiq, inside the Rails app, on Heroku worker dynos. It reads appointment data from MySQL and decides who gets a message, through which channel, and when. Front-desk staff never touch it directly. They notice it only when a patient says they got a message, or did not get one.

## How jobs get scheduled

The general pattern is a periodic scan plus per-appointment jobs. A scheduler job looks for appointments that fall into a reminder window and enqueues one job per appointment. Each job loads the appointment fresh, checks it is still valid, and only then sends. The reading so far suggests this fresh check matters more than anything else in the design, because appointments move and get cancelled between enqueue time and run time.

Some teams schedule jobs far in advance at booking time instead. The team read about that approach and the usual complaint is that rescheduling leaves stale jobs behind in Sidekiq. The scan approach avoids that at the cost of a bit more database reading.

## Idempotency and duplicates

Sidekiq runs jobs at least once. A retry after a crash or a deploy can run the same job twice, so a patient can get two messages. The common fix is to record on the appointment, or in a small sent-log table, that a given reminder was already sent, and to check that record before sending. The check and the write need to be close together, ideally under a row lock or a unique constraint, otherwise two workers can both pass the check.

We also noted that a provider call can succeed while our job still fails afterwards. In that case a retry sends again. Storing the intent before calling out, then marking it done, narrows the gap but does not close it fully.

## Time zones and clinic hours

Clinics sit in different time zones, and reminders should not go out at night. The material we read says to store times in UTC and convert using the clinic's zone when deciding send time. Daylight saving changes are the usual source of off-by-an-hour bugs. Quiet hours are a per-clinic idea, and we have not seen where, if anywhere, they are configured today.

## Interaction with cancellations and the waitlist

When an appointment is cancelled, its pending reminder should not go out. When a slot opens and someone from the waitlist is moved in, that person needs a reminder for the new slot. Both cases depend on the job re-reading state at run time. The waitlist side is covered in [[waitlist-promoter-simultaneous-cancellations]], which has the details on what happens when several cancellations land together. For reminder-worker the point is just that it must not assume the appointment it was enqueued for is the one still sitting in that slot.

## Data handling and FHIR

Appointment data is also exposed through HL7 FHIR resources for other systems. Reminders are a patient-facing channel, so message text should carry the minimum needed: clinic name, time, and how to change the visit. Diagnoses, clinician specialty details and similar health information should stay out of message bodies. We have not confirmed how the FHIR appointment status maps onto the states reminder-worker checks. That mapping is worth reading in the code before anyone changes either side.

## Operations on Heroku

Worker dynos restart regularly and on every deploy. Sidekiq handles shutdown by giving running jobs a short grace period and pushing unfinished ones back to the queue, which is another source of duplicates and a reason to keep jobs short. Reminder jobs should sit in their own queue so a backlog of something heavy does not delay them. Retry behaviour needs thought: too many retries on a failing provider create a burst later, and too few lose reminders silently. Dead jobs should be visible to someone, and today we are not sure who looks.

## Open questions

- Where quiet hours and per-clinic send windows are stored, and who can change them.
- Whether the sent-log is enforced by a database constraint or only by application code.
- How reminders behave when a patient has no usable contact detail, and whether staff see that anywhere.
- Whether the provider reports delivery failures back, and if so what consumes them.
- How reschedules are handled when the new time falls inside the reminder window already.
