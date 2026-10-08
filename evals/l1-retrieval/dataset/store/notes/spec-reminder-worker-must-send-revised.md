---
id: 01JW76PS9Y0XADEYD41ZQNPJG7
created: 2025-05-26T18:06-03:00
---

# reminder-worker spec: SMS reminder lead time

This note replaces the earlier note about "reminder worker must send". The new value: reminder-worker sends the first SMS reminder 48 hours before each appointment. The old lead time was 24 hours and no longer applies.

## Component

reminder-worker is the background component of ClinicSlotter that sends appointment reminders to patients. It runs as Sidekiq jobs inside the Rails app, deployed on Heroku. Its internal codename is `pingd`, so you will see that name in logs, dashboards and older chat. Both names mean the same thing: `pingd` is reminder-worker.

## Requirement

The first SMS reminder goes out 48 hours before the appointment start time. This applies to every appointment that is booked and not cancelled. The earlier 24 hour lead time is replaced, not kept as a second reminder, unless a later note says otherwise.

## Why it matters

Front-desk staff at small clinics need time to refill a slot when a patient cancels. A reminder sent 48 hours ahead gives the patient time to respond and gives the desk more room to rebook the slot with the clinician and room still free.

## Scheduling the job

When an appointment is created, the reminder job is enqueued to run at the appointment time minus 48 hours. The time is computed in the clinic's own time zone, then stored as an absolute timestamp. Do not compute it from server local time.

## Short-notice bookings

If an appointment is booked with less than 48 hours to go, the lead time has already passed. In that case the reminder should be sent right away, or as soon as the job runs, and not skipped. Check this case when changing the scheduling code.

## Rescheduling and cancellation

When an appointment is moved, the old job must be dropped and a new one scheduled from the new start time using the same 48 hour offset. When an appointment is cancelled, the pending reminder must not send. The job should re-check appointment status at run time as a guard.

## Existing queued jobs

Jobs already scheduled under the old 24 hour rule will fire at the old time. Decide whether to reschedule them in bulk when deploying. Left alone, patients with near-term appointments get a later first reminder than the new rule says.

## Data and FHIR

Appointment data comes from the MySQL tables and is exposed through HL7 FHIR resources for outside systems. The reminder lead time is a worker setting and is not part of the FHIR appointment resource. Do not write it into exported resources.

## Configuration

Keep the lead time in one named setting in the worker config so it is changed in one place. Do not repeat the value across jobs, views or message templates. Message text should refer to the appointment date and time, not to how far ahead the reminder was sent.

## Testing

Tests should cover a normal booking, a short-notice booking, a reschedule, a cancellation and a clinic in a different time zone. Freeze time in tests and assert on the scheduled timestamp.

## Open points

Whether a second reminder closer to the visit is wanted has not been settled here. Add it as its own spec if it comes up.
