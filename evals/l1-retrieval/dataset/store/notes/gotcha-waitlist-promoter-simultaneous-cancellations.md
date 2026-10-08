---
id: 01K4EDK5PC9ABFK8VVJGAPF9AC
created: 2025-09-05T23:27-03:00
---

# waitlist-promoter fails on simultaneous cancellations

When two cancellations land at nearly the same moment, waitlist-promoter blows up with `ActiveRecord::StaleObjectError: Attempted to update a stale object: WaitlistEntry` and one of the freed slots never gets offered to anyone. The job does not recover on its own in a way that restores the lost offer, so the slot sits empty until someone notices. Front-desk staff see an open slot and a waitlist that still has people on it.

## Symptom

A Sidekiq job for waitlist-promoter dies with the stale object error above. The other cancellation's job succeeds. Result: two slots were freed, only one was offered. The patient who should have been promoted for the second slot stays on the waitlist as if nothing happened.

## Why it happens

Both cancellations trigger a promotion pass. Each pass loads the same top-ranked WaitlistEntry, decides to promote it, and tries to save. The model uses optimistic locking, so the second writer finds the row has changed since it read it and Rails raises the stale object error. That is the lock doing its job. The bug is what happens afterward: the failed pass is not redone against the next candidate, so its slot is skipped.

## How to recognize it

Look in the Sidekiq dead set or retry set for waitlist-promoter jobs with the error text above. Check whether two cancellations for the same clinic happened within a short window. If the timestamps are close and only one offer went out, this is the cause. It is more likely at small clinics with a short waitlist, because both passes pick the same entry.

## What not to do

Do not remove the optimistic lock to make the error go away. That would let both passes promote the same entry and double-book a patient, which is worse than a missed offer. Do not just rescue the exception and swallow it either; that hides the lost slot.

## Direction for a fix

On a stale object error, reload the candidate list and pick the next eligible entry, then try again for the slot that failed. Each cancelled slot should be tied to its own promotion attempt so a failure can be retried per slot. Another option is to serialize promotion passes per clinic, so two passes never run side by side for the same clinic. Either approach needs a test that fires two cancellations together.

## Reproducing

Set up a clinic with at least two open-eligible waitlist entries and two booked appointments. Cancel both at once, in two threads or two workers, and let Sidekiq run both promotion jobs concurrently. Without the fix, one slot is left unoffered and the stale object error appears in the log. Sequential cancellations do not show it.

## Interim handling

Until a fix ships, check for failed jobs after busy cancellation periods and re-run promotion by hand for the affected clinic. Tell front-desk staff that an empty slot with a non-empty waitlist may be this problem.

## Related areas

The clinician availability and room constraint checks run before an offer is made, so a retry must go through them again rather than reuse the earlier result. Any FHIR messages sent for an offer should only go out for the promotion that actually succeeded.

## Open questions

Whether a Sidekiq retry ever rescues the lost slot depends on timing and is not reliable. Not yet confirmed how often this has happened in production on Heroku; worth counting from the logs before deciding how much effort the fix deserves.
