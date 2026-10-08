---
id: 01KYTDMJDW24BQTB4X42QY5HV9
created: 2026-07-30T18:07-03:00
---

# waitlist-promoter: things to watch out for when changing it

These are general traps in waitlist-promoter, written down so the next change doesn't rediscover them. The component takes a freed slot and offers it to someone waiting. That sounds small, but it sits on top of clinician availability, room constraints, background jobs and outside data, so a change that looks local often isn't. Read this before touching it, and reread the parts that match what you are changing.

## What the component is really responsible for

waitlist-promoter decides who gets a freed slot and moves that person from waiting to booked, or at least to offered. It does not own availability or room rules; it consumes them. Keep that boundary. If you find yourself copying a room check or a clinician hours check into waitlist-promoter, stop and call the shared scheduling code instead. Two copies of a rule drift, and then the front desk sees a promotion that the normal booking screen would have refused.

The reverse also holds. Anything that changes how slots are freed (cancellations, reschedules, clinician time off, room closures) can trigger promotion, so those changes are also changes to waitlist-promoter even if no file in it is touched.

## Ordering and fairness

The order of the waitlist is a business promise to patients and to the staff who explain it at the desk. Small edits to a sort, a query or an index can reorder people without any test noticing, especially when ties exist. Make tie-breaking explicit and stable. Don't depend on the database returning rows in insertion order; MySQL will not promise that once the query plan changes.

Watch for priority rules that have grown over time, such as urgency, appointment type, or how long someone has waited. If you add a new factor, check how it combines with the old ones, and check what happens to people already on the list when the rule changes. Do not silently reshuffle an existing list because of a deploy.

## Concurrency and double promotion

Several things can free the same slot at about the same time, and several workers can pick up promotion work for it. The classic failure is two jobs offering the same slot to two people, or one person promoted into two slots. Rely on a database-level guard (a lock or a uniqueness constraint), not on a check-then-write in Ruby. A Rails validation alone is not enough under load.

When you change locking, think about lock order. Slot, waitlist entry and patient records should always be taken in the same order everywhere, or you get deadlocks that only show up on a busy morning. Keep transactions short and keep outside calls out of them.

## Sidekiq jobs: retries and idempotency

Promotion runs in background jobs, and Sidekiq will retry them. Every job in this area must be safe to run twice, and safe to run late. A retried job should notice that the slot is already taken, that the waitlist entry is already handled, or that the patient has since cancelled, and then do nothing and finish cleanly.

Don't put the whole promotion in one big job if parts of it can fail independently. Separate the decision from the notification, so a failed message doesn't cause a repeated booking, and a repeated booking doesn't cause a repeated message. Pass identifiers into jobs, not loaded objects, and reload state at the start of the job because the world may have changed while it sat in the queue.

Queue delays matter. A promotion computed minutes ago may be stale. Re-validate availability right before committing.

## Stale data and time

Slots are time-based, and clinics work in local time. Be careful with time zones and with daylight saving changes; a slot near a change can be off by an hour if you convert carelessly. Store and compare in one consistent form and convert only at the edges. Check what "now" means in the code, and make it injectable so tests can pin it.

Also think about slots that are too close to start time. Offering a slot that begins almost immediately may be useless or confusing. If there is a cutoff, it should be one named setting, not a literal scattered through the code.

## Offers that expire

If an offer has to be accepted, then expiry is part of the flow and is easy to break. An expired offer must release the slot and move on to the next person, and it must do so exactly once. Test the case where acceptance arrives at about the same time as expiry. Decide which wins and make that decision visible in code, not accidental.

Be careful when changing how offers are marked. Old offers already in the database were written under the old rules, so a change in status meaning needs a data step or a tolerant reader.

## FHIR and outside systems

Appointment and slot state may be mirrored to or read from other systems through HL7 FHIR. A promotion changes appointment status, and outside systems may see that change as a cancellation followed by a new booking, or as an update, depending on how it is sent. Check how the outgoing resource looks before and after your change. Don't change status mapping casually; receiving systems may be strict or may have their own side effects.

Incoming data can be late, repeated or out of order. Don't assume a cancellation arrives before the booking it refers to. Handle repeats without double effects.

## Notifications to patients and staff

Messages are the visible part of this component, and mistakes here reach real people. Check that a patient is never told they have a slot that the system then gives to someone else. Send the confirmation only after the booking is committed, not before. Avoid putting more medical detail than needed in any message; reason for visit and similar fields don't belong there.

Front-desk staff need to see what happened. If promotion changes something automatically, there should be a trace they can read: who was offered, why that person, and what became of it. When you refactor, keep that trace. Losing it makes every support question a code-reading exercise.

## Data and migrations

The waitlist tables can be large and busy. Adding a column or index on MySQL can lock or slow things, and on Heroku you have limited room for long deploy steps. Plan schema changes so old and new code can both run during a deploy, because web and worker processes don't all restart at the same instant. A job enqueued by old code may be run by new code and the other way round, so keep job arguments backward compatible for at least one release.

Be careful with backfills. Run them in batches, make them restartable, and don't let them trigger promotion as a side effect through callbacks. Skip callbacks deliberately when you backfill, and say so in the change.

## Testing it properly

Unit tests for the happy path are not enough here. Cover at least these situations: an empty waitlist, a single entry that no longer fits the slot, entries that fit a clinician but not the room, a patient who is already booked elsewhere at that time, a patient who left the waitlist while a job was queued, a slot taken by a manual booking in between, and a job that runs twice.

Test with more than one worker if you touch locking; a single-threaded test proves nothing about races. Use fixed times. Where you can, test with data shaped like a real small clinic, which usually means few clinicians, few rooms and a waitlist that is mostly the same appointment types.

## Operating and rollout

Prefer to ship changes to waitlist-promoter behind a switch that can be turned off without a deploy, so that if it misbehaves you can stop automatic promotion and let staff handle the list by hand. Make sure that turning it off leaves the data consistent, with no half-finished offers stuck.

After a release, watch the job queue, retry counts and the rate of promotions against what the clinics normally see. A quiet failure here looks like nothing happening: slots stay empty and people stay waiting. Alerts should cover that case as well as errors.

When something goes wrong in production, do not fix it by editing rows by hand unless you have also thought through what jobs and outside messages will then fire. Write down what you changed so the next person can follow it.
