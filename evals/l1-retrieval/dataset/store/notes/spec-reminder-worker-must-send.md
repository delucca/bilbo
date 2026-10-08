---
id: 01JSH1PSCAFS0YDX3KTJ44H2ZH
created: 2025-04-23T07:04-03:00
---

# reminder-worker spec

reminder-worker is the background job component in ClinicSlotter that sends SMS reminders to patients about upcoming outpatient appointments. This note is the spec for when and how it sends them. It is written quickly, so treat gaps as open questions and not as decisions.

The core rule: reminder-worker must send the first SMS reminder 24 hours before each appointment. That is the first reminder, measured back from the appointment start time. Any later reminder is a separate matter and is not defined here.

## Scope

reminder-worker runs as a Sidekiq worker inside the Rails app. It reads appointments from MySQL, builds a message, and hands it to an SMS provider. It does not create or move appointments. The scheduler part of ClinicSlotter owns that, including clinician availability and room constraints. reminder-worker only reacts to what the scheduler has already settled.

Front-desk staff at small clinics are the people who see the effects. They book and move appointments and expect patients to be told without any extra step on their side. So the worker has to be hands-off: no manual trigger needed for the normal case.

## Timing rules

- The first SMS reminder goes out 24 hours before the appointment start. Not the day before at a fixed clock time, and not "morning of". It is relative to each appointment.
- The time is computed from the appointment's stored start time, converted using the clinic's time zone. Clinics may be in different zones, so never compute from server local time.
- If an appointment is booked less than 24 hours ahead, the reminder window has already passed. Decision for now: send nothing late by default and log that it was skipped. Check with the clinics before changing this, since a reminder an hour before may annoy people more than it helps.
- If an appointment is moved, the old scheduled reminder must be cancelled or ignored and a new one computed from the new start time, again 24 hours before.
- If an appointment is cancelled, no reminder is sent. The worker must re-check status right before sending, not only when it was enqueued.

## How scheduling works

The simplest approach is to enqueue a Sidekiq job at booking time with a scheduled run time. This has a known weakness: when appointments move, stale jobs stay in the queue. Because of that, the job should carry only the appointment reference and, when it runs, load the current record and decide whether to send. If the current start time no longer matches the intended reminder time, the job exits quietly.

An alternative is a periodic sweep that looks for appointments entering the reminder window. It is more robust against drift and restarts, and it also covers Heroku dyno restarts that lose in-memory state. Sidekiq's Redis-backed schedule survives restarts, but a sweep is a decent safety net. Current preference: scheduled jobs as the main path, with a sweep as a backstop if we see missed sends.

Sending must be idempotent. Record that the first reminder was sent for an appointment, and check that record before sending. Sidekiq retries jobs, and a retry after a partial failure must not text a patient twice.

```ruby
# sketch only
class ReminderWorker
  include Sidekiq::Worker

  def perform(appointment_id)
    # reload, check status, check already-sent, then send
  end
end
```

## Message content and data

Keep the message short and plain: clinic name, date, time, and how to reach the clinic to change it. No clinical detail goes in an SMS. Treat the phone number and the appointment as personal health data. Do not write message bodies or numbers to general logs; log the appointment reference and the outcome instead.

Appointment data is also exposed through HL7 FHIR resources elsewhere in the app. reminder-worker does not need to read FHIR directly; it uses the local appointment records. If the FHIR representation and the local record ever disagree about the start time, the local record the scheduler writes is the source for reminders.

Patients without a usable mobile number are skipped, and the skip is logged so front-desk staff can see it if a screen for that exists later.

## Failures and open questions

- Provider errors: retry with backoff through Sidekiq, then give up after a bounded number of attempts and mark the reminder as failed. Do not retry past the appointment start.
- Opt-out: patients who opted out of SMS get nothing. Where that flag lives needs confirming before this ships.
- Rate limits: a busy clinic can have many appointments in the same hour. Spread sends if the provider throttles us.
- Testing: use a fake clock in tests for the 24 hours rule, and cover moved, cancelled and late-booked appointments, plus a duplicate retry.
- Open: whether a second reminder exists, and whether clinics can configure the lead time. For now the lead time is fixed at 24 hours for the first reminder.
