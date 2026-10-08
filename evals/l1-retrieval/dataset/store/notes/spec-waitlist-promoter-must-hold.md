---
id: 01K8T3D6Z2Y04JV7RP4Q68J06F
created: 2025-10-30T05:23-03:00
---

# waitlist-promoter spec

waitlist-promoter is the part of ClinicSlotter that turns a freed appointment slot into an offer for someone on the waitlist. This note is the working spec. The main rule is that waitlist-promoter must hold an offered slot for 15 minutes before offering it to the next entry. Everything else here exists to make that rule safe and predictable for front-desk staff at small clinics.

The note is written for whoever touches the promoter next, whether an engineer or a coding agent. It says what the component does, what it must not do, and where the traps are. It deliberately leaves out exact config names, queue names and file paths, because those drift. Look them up in the code when you need them.

## Purpose and scope

Clinics cancel and reschedule all day. A freed slot that nobody fills is lost clinician time. A freed slot that gets filled badly, for example by someone who cannot reach the clinic in time or who was never really asked, creates a no-show and a confused front desk. waitlist-promoter sits between those two failures. It watches for slots that become free, picks the best waiting entry, offers the slot, and waits for an answer.

In scope: choosing the next waitlist entry for a freed slot, creating the offer, holding the slot while the offer is open, moving to the next entry when the hold ends without an acceptance, and recording what happened so staff can see it.

Out of scope: building the original schedule, deciding clinician availability, and defining room constraints. Those belong to the scheduling core. waitlist-promoter consumes their answers and never overrides them. It also does not send the message itself in any channel-specific way; it hands an offer to the notification path and reacts to the reply.

The component runs as background work in Sidekiq on top of the Rails app, with MySQL as the source of truth. Nothing about the hold lives only in memory.

## The hold rule

The core rule: when a slot is offered to a waitlist entry, the slot is held for that entry for 15 minutes. During those 15 minutes the slot is not offered to anyone else, and it is not bookable by the front desk through the normal booking path without an explicit override. Only after the 15 minutes have passed without an acceptance does waitlist-promoter offer the slot to the next entry.

A few consequences worth stating plainly:

- The hold is per offer, not per slot. If the first entry declines or lets the hold lapse, the next entry gets its own fresh hold of 15 minutes.
- The clock starts when the offer is recorded as sent, not when the slot was freed and not when the job was enqueued. If the notification path fails before sending, the clock must not start.
- An explicit decline from the entry ends the hold early. The slot may then go to the next entry right away. The 15 minutes is a maximum wait on silence, not a mandatory delay after a refusal.
- An acceptance inside the hold converts the slot into a real appointment and ends the promotion for that slot.
- If a front-desk user cancels the offer by hand, the hold ends and the slot returns to the pool for the next pick.

The value is a product decision, not a technical limit. It was chosen so that someone away from their phone for a short while can still respond, while the clinic does not leave a slot idle for long. If it ever needs to change, change it in one place and update this note; do not scatter copies.

## Offer lifecycle

An offer moves through a small set of states. Keep the set small, because every extra state is another path to test.

- Pending: the offer has been created but not yet confirmed as sent.
- Held: the offer was sent and the hold is running.
- Accepted: the entry said yes inside the hold and the appointment was created.
- Declined: the entry said no inside the hold.
- Expired: the hold ran out with no answer.
- Cancelled: staff or the system withdrew the offer, for example because the slot was taken another way or the clinician became unavailable.

Only Held offers block the slot. Pending is a short transitional state and must never be left standing; a sweeper should treat a stale Pending as a failure to report, not as a hold. Accepted, Declined, Expired and Cancelled are terminal. Terminal offers are kept as history and never edited afterward, so staff can later answer the question of why a given patient was or was not offered a slot.

Transitions out of Held are the dangerous ones, because an acceptance and an expiry can arrive at nearly the same moment. See the section on races.

## Choosing the next entry

When a slot becomes free, waitlist-promoter builds a candidate list from the waitlist and picks from it. The pick must be deterministic given the same data, so that two workers looking at the same slot would choose the same entry.

Filtering comes first. An entry is a candidate only if the slot fits what the entry asked for: the right kind of visit, an acceptable clinician or an acceptable alternative, an acceptable time window, and a room that satisfies the visit's room needs. The entry must also still be active, meaning not already booked elsewhere, not withdrawn, and not currently holding another open offer for a conflicting time.

Ordering comes second. The order favors entries that have waited longest, with clinical urgency taking priority where the clinic has recorded it. Ties break on a stable key so the result does not flip between runs. Keep the ordering logic in one place so changes to it are visible in one diff.

The promoter never skips an entry silently. If an entry is filtered out, there should be a recorded reason available for debugging, even if that reason is only logged and not shown to staff.

One entry should not hold offers for overlapping slots at once. Otherwise a patient could accept both and the clinic would have to undo one. If an entry already has an open offer, treat it as ineligible for another overlapping slot until that offer ends.

## Triggers

There are a few things that cause waitlist-promoter to look for work.

A slot freed by cancellation is the common one. The cancellation path enqueues a job for the promoter once the cancellation is committed. The job must run after commit, never inside the transaction, so a rolled-back cancellation cannot produce an offer for a slot that is still taken.

A hold ending is the second. When a hold expires, a delayed job fires and moves to the next entry. This is how the 15 minutes becomes real behavior. The delayed job must be idempotent, because it can fire after the offer has already reached a terminal state.

A new waitlist entry is the third. When someone joins the waitlist and a matching free slot already exists, the promoter may offer it. This must follow the same hold rule as any other offer.

A periodic sweep is the safety net. It looks for free slots with eligible entries and no open offer, and for Held offers whose hold has clearly ended without the delayed job running. The sweep exists because queued jobs get lost, deploys restart workers, and Heroku dynos cycle. The sweep should be cheap and safe to run often, and must go through the same code path as the event-driven triggers instead of having its own logic.

## Concurrency and races

The promoter runs in several Sidekiq workers and shares MySQL with the web process, so assume that two things will try to act on the same slot at the same time.

The rule is that the database decides. Taking a hold means writing the offer row and marking the slot as held in one transaction, under a lock on the slot row, so only one offer can win. A worker that loses the lock re-reads and finds the slot already held, then exits without doing anything. Do not rely on a Redis lock or an in-process check as the only guard.

The acceptance-versus-expiry race needs a clear winner. Both the acceptance handler and the expiry job should lock the offer row, re-check that it is still Held, and only then transition it. If the acceptance gets there first, the expiry job sees a terminal state and does nothing. If the expiry gets there first, a late acceptance is treated as too late: the patient is told the slot is gone, and may be put back on the waitlist. Do not try to honor a late acceptance by quietly bumping the next entry, because that creates confusing messages.

There is a small gray area around the exact end of the hold. Decide it once, in code, using the server clock in the database or the app consistently, and write a test for it. Do not compare a client-provided time against the hold end.

Front-desk manual booking is the other competing writer. While a slot is held, the normal booking path should refuse and say who holds it and until when. An explicit override by staff is allowed, and it must cancel the open offer cleanly, with the entry told that the slot is no longer available.

## Jobs, retries and idempotency

Sidekiq retries failed jobs, so every job in this component must be safe to run more than once. The practical approach is to make each job take an identifier for the thing it acts on, load the current state, and decide what to do from that state, not from the arguments it was enqueued with.

The expiry job is the best example. It carries the offer identifier only. When it runs it loads the offer, and if the offer is not Held it returns. If it is Held and the hold has ended, it marks it Expired and triggers the next pick. If it is Held and the hold has not ended yet, for example because the offer was somehow extended, it reschedules itself instead of acting early.

Do not encode the 15 minutes into the job by assuming it ran at the right moment. Jobs run late under load. Compute the end of the hold from the stored send time, and compare with the current time when the job actually runs.

Retries should use backoff, and a job that keeps failing should end up somewhere visible instead of disappearing. A dead job for an expiry means a slot may be stuck in a hold; the sweep is what recovers it, but someone should also see the alert.

Keep each job small. Picking an entry, creating an offer, and sending the notification are separate steps with separate failure modes. If sending fails, the offer should not be left looking Held.

## Data model notes

The stored shape matters more than the exact column names, which can change. The promoter needs these facts persisted in MySQL.

For each offer: which slot, which waitlist entry, the state, when it was created, when it was sent, when the hold ends, when it reached a terminal state, and the reason for a terminal state such as a decline, an expiry, or a cancellation by staff. Storing the hold end explicitly is better than recomputing it, because it keeps old offers explainable if the rule changes later.

For each slot: whether it is currently held, and by which offer. This can be derived from offers, but a direct marker makes the booking path check cheap and makes the lock simple. If both exist, keep them consistent in the same transaction, and have the sweep repair any mismatch rather than trusting either blindly.

For each waitlist entry: its preferences, its position information, its status, and a link to any open offer. Do not delete entries when they are fulfilled or withdrawn; mark them, so history stays readable.

Use database constraints where possible to back up the rules. A uniqueness guarantee that only one open offer exists per slot catches bugs that application checks miss. The same applies to one open overlapping offer per entry, to the extent it can be expressed.

Timestamps should be stored in UTC and shown in the clinic's local time. Clinics in different zones are normal, and daylight saving changes have bitten scheduling code before. A hold is a duration, so it should be added to a UTC instant, not to a local wall-clock time.

## FHIR and external systems

ClinicSlotter exchanges scheduling data using HL7 FHIR where the clinic has an integration. The promoter touches that boundary in a limited way, and the limit is intentional.

A slot that is held is still a free slot as far as an outside system is concerned unless we say otherwise. Decide with each integration whether a held slot should be published as busy during the hold. The safer default is to publish it as busy, so an outside booking cannot take it, and to republish it as free if the hold ends without an acceptance and no further offer is made. If an integration cannot represent this, the hold is only enforced inside ClinicSlotter, and that gap should be written down for the clinic.

When an offer is accepted, the resulting appointment goes out through the same path as any other new appointment. The promoter should not build its own FHIR resources. It asks the scheduling core to create the appointment and lets the normal outbound flow do its work.

Incoming changes from outside, such as an appointment cancelled in another system, may free a slot. Treat that exactly like a local cancellation: it triggers the promoter after the change is committed. Do not let a malformed incoming message trigger offers; validate first.

Patient data in offers is health-related. Keep logs free of names and contact details. Log identifiers and states, not content.

## Notifications and what staff see

The promoter creates the offer and passes it to the notification path, which is responsible for reaching the patient. The promoter cares about only two outcomes: the message was handed off successfully, which starts the hold, or it failed, which means no hold and a decision about what to do next.

On failure, the promoter should retry the send a limited number of times and then give up on that entry for this slot, record why, and move to the next entry. Leaving a slot stuck on an entry that cannot be reached defeats the purpose.

The offer message must say that the slot is held for a limited time and when that time ends, so the patient knows the clock. The wording belongs to the notification templates, but the end time shown must come from the stored hold end, not recomputed separately in the template.

Front-desk staff need a simple view. For each open slot they should see whether it is free, held, or filled; if held, for whom (as far as privacy allows on that screen) and until when; and the recent history of offers on it. They should be able to cancel an open offer and to offer the slot to a specific entry out of order. Both actions must go through the same state transitions as automatic ones, never edit rows directly.

Staff are busy and the clinics are small. Do not add settings that nobody will understand. A clinic can turn the promoter off entirely or on; anything more granular needs a real request behind it.

## Failure modes and recovery

Some things will go wrong, and the design should make them boring.

A worker dies mid-way after creating an offer but before sending. The offer stays Pending, no hold is running, and the slot is marked as held only if the transaction did that. The sweep notices a stale Pending and either finishes the send or cancels the offer and re-picks. It must never leave the slot marked held without a sent offer behind it.

A worker dies after sending but before scheduling the expiry job. The hold is real but nothing will end it. The sweep finds Held offers past their hold end and expires them. This is why the sweep must exist and why hold end is stored.

The queue backs up. Expiry jobs run late, so a slot sits held longer than intended. That is annoying but safe. The reverse, an early expiry, is not acceptable, which is why jobs compare against stored times when they run.

A deploy or dyno restart on Heroku interrupts jobs. Graceful shutdown should let jobs finish or return them to the queue. Jobs that were in flight are retried, and idempotency covers the repeat.

The database is briefly unavailable or a lock times out. The job fails and retries. Do not catch lock timeouts and carry on as if the lock was taken.

The waitlist is empty when a slot frees up. Nothing happens, and that is correct. The slot simply stays free for normal booking. No offer rows should be made for nobody.

Every entry in the list declines or expires. The slot goes back to the free pool, and the history shows the offers made. A later new waitlist entry or the sweep can pick it up again if it is still in the future.

## Testing guidance

Tests for this component should lean on the rules, not on implementation details.

Cover the hold rule directly: an offered slot is not offered to the next entry before the 15 minutes pass, and is offered after. Use controllable time instead of sleeping. Cover the early-end cases: a decline ends the hold early, a cancellation by staff ends it, an acceptance converts it.

Cover the races with real database transactions, not mocks. Two workers picking for the same slot should yield one offer. An acceptance and an expiry on the same offer should yield exactly one terminal state, whichever got there first, and in both orders.

Cover idempotency by running each job twice and checking the second run changes nothing. Cover the sweep by building broken states, such as a stale Pending, a Held offer past its end, and a slot marked held with no offer, and checking each gets repaired.

Cover the filters: an entry whose preferences do not fit the slot is never offered it; an entry with an open overlapping offer is skipped; a withdrawn entry is skipped. Cover the ordering with ties so the result is stable.

Cover the boundary with the booking path: while held, normal booking refuses; with an override, the offer is cancelled and the entry is told.

Time zone and daylight saving cases deserve at least one test each, since the hold is a duration added to a UTC instant and should not move when clocks change.

## Open questions and things to watch

These are not settled and should not be treated as decided.

- Whether the hold should be allowed to differ by visit type. Right now there is a single rule. Some visits may justify a shorter or longer wait, but that would be a new product decision.
- Whether a patient who lets a hold lapse should lose priority or keep their place. The current lean is to keep their place, since silence is not a refusal, but this has not been confirmed with clinics.
- Whether to offer to more than one entry at once for very short-notice slots. That would break the one-at-a-time hold rule, so it needs its own design and should not be slipped in as a tweak.
- How held slots should appear in each outside integration, as covered in the FHIR section. Each integration needs an explicit answer.
- Whether the sweep should alert staff when it repairs something, or only log. Repairs mean something upstream failed, so some visibility is probably right.

If you change any behavior described above, update this note in the same change. The most likely drift is the hold rule itself: keep the 15 minutes stated here in agreement with the single place in code that defines it.
