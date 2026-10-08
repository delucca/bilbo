---
id: 01KMFP7E6SS5VS1MMTHB9K2BEN
created: 2026-03-24T07:28-03:00
sources:
  - "code: app/jobs/waitlist_promoter.rb"
---

# waitlist-promoter design

waitlist-promoter is the part of ClinicSlotter that fills a slot again after someone cancels. It is the Sidekiq job `WaitlistPromoter`. A cancellation triggers it. It picks the first waitlist entry ordered by priority and then `created_at`. Everything below is the reasoning around that rule and the way the job is meant to behave. It is written quickly, so treat the wording as notes and not as a spec. Front-desk staff at small clinics use this app. They need the freed slot to go to someone sensible without having to phone around first. They also need to be able to see why a given person was chosen.

## Purpose

A small clinic loses real money and clinician time when a cancelled slot stays empty. Staff used to scan the waitlist by hand after each cancellation. The promoter does that scan. It looks at the freed slot, finds the best waiting entry that fits, and moves that entry into the slot. The goal is not to be clever. The goal is that the result is predictable enough for front-desk staff to explain to a patient who asks why someone else got the slot.

## Trigger

The job is enqueued when an appointment is cancelled. The cancellation code path does the enqueue and passes only what the job needs to find the freed slot again. The job loads fresh state when it runs and does not trust anything that was true at enqueue time. Cancellation can be done by staff or by the patient through whatever channel the clinic uses. Both paths end in the same enqueue. Nothing else enqueues the job today. If we add a nightly sweep later, it should call the same job, not a copy of its logic.

## Selection rule

The job picks the first entry ordered by priority and then `created_at`. Priority comes first. Among entries with equal priority, the one that was created earliest wins. That is the whole rule for choosing between candidates. The order is applied to the candidates that are eligible for this particular slot, not to the whole waitlist. A high-priority entry that cannot use the slot is skipped, and the next entry in the same order is tried. We do not reorder the list to make a skipped entry jump ahead later.

## Ordering details

Priority is a coarse value set by staff when they add someone, for example for clinical urgency. It is not computed by the job. `created_at` is the time the waitlist entry was made. It is used as the tie-breaker because it is the easiest thing to defend to a patient: who asked first. We do not use the appointment's own timestamps for ordering. Editing an entry should not change `created_at`, or the tie-break becomes meaningless. If someone needs to jump the queue, staff should raise priority instead of recreating the entry.

## What counts as an entry

An entry is a patient asking for an earlier or any available slot with a given clinician or service. Entries that are already fulfilled, withdrawn, or expired are not candidates. An entry may carry preferences such as a time of day or a clinician. The job treats those as filters, not as ordering inputs. A patient who already has an appointment that the entry is meant to replace is handled as a move: the old appointment is released when the new one is booked. That release can itself trigger the job again, which is expected and is covered under concurrency.

## Slot being offered

The slot is described by clinician, room, and time window, taken from the cancelled appointment. The job re-reads the slot to confirm it is still free. If another booking took it between the cancellation and the job run, the job stops quietly. It does not search for a different slot on its own. One cancellation frees one slot, and the job handles exactly that slot and nothing else.

## Constraints checked before promoting

Before moving an entry into the slot, the job checks the same constraints that normal booking checks. It should reuse the booking validation, not duplicate it. If the two ever drift, the promoter will create appointments that staff could not have made by hand, and that is a bug. The checks cover clinician availability, room constraints, the appointment type and its length, and any clinic rule about double booking a patient. A failed check means the entry is skipped for this slot only. The entry stays on the waitlist.

## Clinician availability

A freed slot is only useful if the clinician is still working then. Schedules change, and a cancellation can arrive after a clinician's day was shortened or blocked. The job reads current availability at run time. If the clinician is no longer available for the window, no entry is promoted and the slot is left alone. We do not try to be helpful by offering a nearby time. That would turn a simple rule into a search problem, and staff would no longer be able to predict the outcome.

## Room constraints

Rooms are shared, and some appointment types need a particular kind of room. The slot carries its room, and the job checks that the room is still valid for the new entry's appointment type. If the entry needs a different room type, it does not fit, and it is skipped. The job does not reassign rooms to make something fit. Room swaps are a staff decision. The same applies when a room was taken out of service after the original booking was made.

## Job behavior in Sidekiq

`WaitlistPromoter` runs in a normal Sidekiq queue, not a special one. It is short. It does a handful of reads and at most one write path for the booking. It should not call out to slow external systems while holding a database transaction. Notifications to the patient are sent after the booking is saved, from a separate job, so that a slow messaging service cannot hold up promotion or cause it to be retried for the wrong reason.

## Idempotency and retries

Sidekiq will retry on failure, and a job can also be delivered more than once. The job must be safe to run again for the same cancellation. On a second run it should find the slot already taken, or the entry already fulfilled, and do nothing. That is the main reason it re-reads state at the start instead of trusting arguments. We prefer a no-op over an error when the situation has simply moved on. Real errors, such as a database outage, should raise so the retry happens.

## Concurrency

Two cancellations close together can trigger two runs that look at the same waitlist. Without care, both could pick the same entry. The job takes a lock on the entry or the slot as it books, and re-checks inside that lock that both are still available. The loser sees the change and ends as a no-op, or tries the next candidate if the slot is still free. The move case described earlier also relies on this: releasing the old appointment enqueues a new run, and that run must not fight with the one that caused it.

## Database access

The candidate query is the hot part. It filters entries by eligibility and then orders by priority and `created_at`, so it should be able to use an index. We run on MySQL, and the shape of the appointments table matters for the slot re-check. Index choices for that table are in [[appointments-table-composite-index]]. Keep the candidate query simple and avoid loading the whole waitlist into memory. The job needs only the first fitting entry, so it should stop once it finds one.

## FHIR side

ClinicSlotter exchanges data with other systems through HL7 FHIR. A promotion changes an appointment, so the outbound FHIR representation has to follow. The job does not build FHIR resources itself. It saves the booking, and the normal export path picks up the change. The cancelled appointment and the new one both show up as ordinary state changes. We do not want a special promoted status leaking into the exchange format, because receiving systems would not know what to do with it.

## Failure cases

The common non-failures are these: no entries on the waitlist, no entry that fits, slot already taken, clinician no longer available. All of them end the job without an error and without noise. The real failures are infrastructure ones, which retry. If an entry keeps failing validation in a way that looks like bad data, the job skips it and logs enough to find it later. One broken entry must not block everyone behind it. Logs should say which entry was chosen and why the earlier ones were skipped, since staff will ask.

## Things not done on purpose

The job does not overbook. It does not split a slot. It does not contact the patient before booking, so there is no offer and accept step yet. It does not weigh distance, no-show history, or anything else beyond priority and `created_at`. It does not run on Heroku in a special dyno type. Each of these has come up and was left out to keep the behavior easy to explain. Adding any of them changes the rule that staff rely on, so do it only with a clear decision recorded.

## Open questions

Should a patient be asked to confirm before the booking becomes final? Right now the promotion is immediate, which suits clinics that phone anyway but may surprise patients. Should priority have a documented meaning per clinic, or stay free-form? Should skipped high-priority entries be surfaced to staff so they can call by hand? None of these is decided. If one is, update this note and keep the selection rule at the top accurate, since the rest depends on it.
