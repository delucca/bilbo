---
id: 01JTCSX4EJXXTB74XKMQH1HP2J
created: 2025-05-04T01:47-03:00
---

# reminder-worker: options survey

This is a survey of the general options we looked at for reminder-worker, the part of ClinicSlotter that tells patients about upcoming appointments. Nothing here is settled. It is a map of the choices so the next session does not redo the reading. Front-desk staff at small clinics are the users, so the main worry is a reminder that goes out wrong or goes out twice, not raw throughput.

## What the worker has to do

reminder-worker reads booked appointments, works out who should be told and when, and sends the message through some channel. It also has to cope with changes: an appointment moved, cancelled, or reassigned to another clinician or room after the reminder was already queued. That last part is the hard one. A reminder for a slot that no longer exists is worse than no reminder.

Loose requirements we keep coming back to:

- Reminders must be idempotent. A retry should never produce a second message to the patient.
- A change to an appointment must be able to cancel or replace a pending reminder.
- Clinic staff should be able to see what was sent and what failed, without reading logs.
- It runs on Heroku, so anything that assumes a long-lived box or a local disk is out.
- Patient contact data is sensitive. Message bodies should carry as little clinical detail as possible.

## Scheduling options

### Sidekiq delayed jobs

Enqueue a job at booking time with a scheduled run time. This is the obvious fit because Sidekiq is already in the stack. The cost is that a scheduled job is a copy of the appointment facts at enqueue time. If the appointment changes, the job has to find out. Two ways to handle that: store the job id on the appointment and delete the job on change, or let the job re-read the appointment when it runs and quietly do nothing if the state no longer matches. The second is more forgiving. The first leaves orphans when a delete fails.

Another point against far-future scheduled jobs: they sit in Redis for a long time, and a Redis flush or a bad deploy of the queue store loses them with no record in MySQL. Fine for short horizons, uncomfortable for long ones.

### Periodic sweep

A recurring job scans MySQL for appointments that fall inside a reminder window and have no sent record. Nothing is scheduled ahead of time, so there is nothing to cancel. Changes are picked up for free because the sweep reads current state. The downsides are window edges (missing or double-sending near the boundary), query cost as the table grows, and coarser timing. Needs a sent-marker written in the same transaction, or close to it, as the decision to send.

Sidekiq cron-style gems or the Heroku scheduler could drive it. The Heroku scheduler is simple but not very precise and does not retry; a cron gem inside Sidekiq gets retries but depends on the worker process being up at the right moment.

### Hybrid

Sweep to find candidates, then enqueue a send job per candidate with a unique key. The sweep gives correctness against changes; the per-item job gives retries and isolation, so one bad phone number does not stall the batch. This looks like the strongest shape so far, but it has two moving parts instead of one.

## Delivery channels

- SMS through a provider API. Highest open rate for this audience, but per-message cost, opt-out handling, and carrier filtering all become our problem or the provider's.
- Email through a transactional mail service or the framework mailer. Cheap and easy to template. Lower chance the patient sees it in time. Spam placement is a risk.
- Voice calls. Some older patients prefer them. More cost and more failure modes (no answer, voicemail). Probably out of scope for a first version.
- Patient portal or push. Only useful if patients have the app. Small clinics often do not.

Whatever we pick, put the channel behind a small adapter so reminder-worker does not know provider details. That also makes it easier to test with a fake and to switch providers later.

## Idempotency and state

Options for making a send safe to retry:

1. A reminders table with a unique constraint on appointment plus reminder kind. Insert before sending; if the insert fails, skip. Simple, lives in MySQL, survives Redis loss.
2. Sidekiq unique-job support. Convenient, but it guards against duplicate enqueue, not duplicate send after a crash between provider call and ack.
3. Provider-side idempotency keys, where the provider supports them. Good as a second layer, not a replacement for our own record.

Option 1 is the baseline. The awkward case is a crash after the provider accepted the message but before we mark it sent. We either accept a rare duplicate or mark as "attempting" first and accept a rare miss. Staff would probably rather get a duplicate than a miss, but that is a product call to confirm with them.

A status field with a few states (pending, sent, failed, cancelled) also gives the staff view for free.

## FHIR angle

ClinicSlotter speaks HL7 FHIR, so appointments may arrive or change through that path rather than only through the UI. If reminder-worker hangs off Rails model callbacks, FHIR-driven changes need to go through the same models or they will be missed. Worth checking that every write path touches the same code. An alternative is to react to an internal event published on any appointment change, whatever its origin. Related: FHIR has communication-style resources that could record the reminder itself, but mapping our own reminder state onto them looks like extra work with little payoff for now.

## Time zones and quiet hours

Reminders are computed relative to the clinic's local time, not the server's. Daylight saving changes can shift a window by an hour. Store appointment times in UTC, keep the clinic zone on the clinic record, and compute the send time at the last moment. Also consider quiet hours so nobody gets a text at night because a window landed badly.

## Failure handling

- Retries with backoff for transient provider errors; no retry for permanent ones such as an invalid number.
- A dead-letter view that staff or an engineer can look at.
- An alert when the failure rate climbs, rather than on every single failure.
- Do not retry past the point where the appointment has already happened.

A sketch of the worker shape under the hybrid option, only to fix the idea:

```ruby
class ReminderWorker
  include Sidekiq::Job

  def perform(appointment_id)
    # re-read current state; skip if cancelled or already sent
  end
end
```

## Open questions

- Which channel do the clinics actually want, and who pays for messages?
- Duplicate versus miss: which does staff prefer on an ambiguous failure?
- Do patients need to reply to confirm or cancel? That pulls in inbound handling and changes the design a lot.
- How many reminders per appointment, and can clinics configure the timing?
- Is there a legal or consent requirement for contacting patients that limits content or channel?

## Leaning

No decision yet. The leaning is hybrid scheduling, a MySQL reminders table as the source of truth, and a channel adapter, with SMS as the first provider to try. Revisit once the open questions above have answers.
