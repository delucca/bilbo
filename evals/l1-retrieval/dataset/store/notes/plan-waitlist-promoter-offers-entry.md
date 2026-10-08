---
id: 01KT6HDVM3140R5GSHDW0RF55V
created: 2026-06-03T07:45-03:00
---

# waitlist-promoter offer cap plan

Plan for limiting how many times waitlist-promoter can offer a freed slot to the same waitlist entry in a single day. The cap is set with the environment variable `WAITLIST_MAX_OFFERS=3`, meaning each waitlist entry gets at most three offers per day from waitlist-promoter, and after that the entry is skipped until the next day. This note covers the reasoning, where the change goes, the order of work, the edge cases I already know about, and what is still open. It is a plan, not a record of finished work. Nothing here has been built yet.

## Why a cap

Front-desk staff at small clinics use ClinicSlotter, and they have complained that some patients on the waitlist get pinged over and over. The pattern is easy to reason about. A cancellation frees a slot. waitlist-promoter picks the best matching waitlist entry and sends an offer. The patient does not answer in time, or declines, and the slot goes back to the pool. Then another cancellation happens, and the same entry is still the best match because nothing in the ranking penalizes a recent offer. On a busy day with several cancellations the same person can be offered slot after slot. That is annoying for the patient, it fills the front desk's inbox with declined offers, and it makes the entry look like a bad candidate when really it is just top-ranked.

The cap fixes the symptom directly. It does not change ranking. It only says: this entry has had its share of offers today, move on to the next candidate. The value is three because that is enough for a patient to see a couple of alternatives and still have a chance at a good time, without turning into spam. It is an environment variable so a deployment can tune it without a code change, and so staging can set it low to exercise the behavior quickly.

Things the cap is not meant to do:

- It is not a fairness mechanism across patients. Ranking still decides who is first.
- It is not a rate limit on the whole system. Other entries are unaffected.
- It is not a replacement for the patient declining a slot permanently. A decline of a specific slot is already handled elsewhere and should keep working the same way.

## Where it lives

The promotion logic runs as a Sidekiq job in the Rails app. waitlist-promoter is triggered when a slot is released, either by a cancellation or by a clinician changing availability, and it walks the waitlist for candidates. The new check belongs in the candidate selection step, not in the sending step. If the check sat in the sender, the job would already have picked an entry, reserved the slot for it, and then have to undo that. Putting it in selection means a capped entry is simply never chosen.

To know how many offers an entry has received today, we need a count. Two options were considered.

The first is to count rows in the existing offers table, filtering by waitlist entry and by the clinic's local day. This needs no new storage and cannot drift from reality, since the offer record is the source of truth. The cost is one extra indexed query per candidate. With small clinics and short waitlists that is fine. Check that the offers table has an index that supports the lookup by entry and creation time; if it does not, add one in a migration. MySQL handles this well as long as the index exists.

The second is a counter in Redis keyed by entry and day, with an expiry. It is faster but can disagree with the database after a crash or a retry, and it adds a second place to look when debugging. I am not going with it. If the query turns out to be slow in practice we can revisit.

Decision for now: count from the offers table. Keep the counting in one small query object or model scope so the job stays readable and the same scope can be reused by a report or an admin screen later.

## The day boundary

The word day needs a definition, because clinics are in different time zones and Heroku dynos run in UTC. A UTC day would reset the cap in the middle of a clinic's afternoon for some locations, which defeats the purpose. The day must be the clinic's local calendar day, using the time zone already stored for the clinic. The scope should take the start and end of the local day, converted for the query, rather than comparing dates as strings.

Two details to remember here. Daylight saving changes make some local days shorter or longer, so compute boundaries from the zone and not by adding a fixed span to midnight. And an offer created just before midnight local time counts toward the day it was created in, not the day it expires in. An offer that is still pending across midnight does not count again on the new day.

## What counts as an offer

This needs to be pinned down before coding, because it changes the numbers a lot.

Counts toward the cap:

- Any offer that waitlist-promoter created and sent to the entry, whatever its outcome: accepted, declined, expired, or still pending.

Does not count:

- An offer that was created but failed to send because of a delivery error on our side. The patient never saw it, so it should not use up their allowance. This needs the offer record to carry a clear status for send failure. If the current statuses do not distinguish it, add one rather than guessing from timestamps.
- Manual bookings made by front-desk staff for that patient. Those are staff decisions, not promotions.
- Offers withdrawn because the slot was taken by someone else before the patient could see it. This is a gray area. Lean toward not counting them if the notification was never delivered, counting them if it was. Confirm with whoever owns the notification path.

An accepted offer ends the story for that entry anyway, since the entry leaves the waitlist or moves to booked. So in practice the cap is about declines, expiries, and pending offers.

## Configuration handling

Read the value of the variable once at boot into an application config object, not on every call to the job, so tests can stub it cleanly and so the behavior is the same for the whole life of a process. Parse it as an integer. Rules for odd values:

- Missing: fall back to the default of three. The default should be defined in code, so an install with no variable set still gets protection.
- Not a number, empty, or negative: log a warning at boot and use the default. Do not crash the app over this.
- Zero: this could be read as a deliberate setting that means no offers at all, but that silently disables promotion. Decision: do not allow zero. Log a warning and use the default. If someone really wants to pause promotion they should use a proper switch, not this variable.

On Heroku the variable is set per app with the usual config commands, and changing it restarts the dynos, which is what we want. Document it in the deployment readme next to the other variables, with the plain statement that it is the per-entry, per-day offer cap for waitlist-promoter and that its default is three.

## Order of work

Do these in order, each as its own small change so review stays easy.

- Add or confirm the offer status that separates sent from failed-to-send. Backfill is probably not needed if old rows can be treated as sent, but check.
- Add the supporting index if the lookup by entry and time is not covered. Try it on a copy of production-sized data first, since MySQL index builds on a large table can lock or slow things.
- Add the config reader with the default and the validation rules above, with unit tests for each odd value.
- Add the counting scope with the local day boundaries, with tests around midnight and around a daylight saving change.
- Wire the check into candidate selection in waitlist-promoter so capped entries are filtered out before ranking picks a winner. Filter before ranking, not after, so the next best candidate is chosen properly rather than the job returning nothing.
- Add logging for each skip: entry identifier, count so far, cap value. No patient names or other personal data in the log line.
- Add a metric or at least a log-based count of how often the cap fires, so we can see whether three is right.
- Update the readme and the runbook.

## Edge cases

Concurrency matters most. Two Sidekiq workers can handle two cancellations at nearly the same moment, both read a count of two for the same entry, and both send an offer, leaving the entry over the cap. Options: take a row lock on the waitlist entry while checking and creating the offer, or use a database-level guard. The row lock inside the transaction that creates the offer is the simplest and fits how the app already works with MySQL. Keep the locked section short: count, create the offer record, commit, then enqueue the actual send after commit. Do not hold the lock during any network call to a notification provider.

Retries are the second concern. Sidekiq will retry a failed job. A retry must not create a second offer for the same slot and entry. The offer creation should be idempotent on the pair of slot and entry, so a retry finds the existing record and does not count twice. If the existing code does not already guard this, fixing that is part of this work, not a follow-up.

When every candidate is capped, the slot stays open and unfilled. That is acceptable. Do not loosen the cap to fill a slot. But surface it somewhere: front-desk staff should be able to see that the slot is open and that waitlisted patients were skipped because of the cap, otherwise it looks like the system is broken. A short note on the schedule view or in the log is enough for the first version.

Entries that are added to the waitlist for several clinicians or rooms: the cap applies per entry, across all of them, since the patient experiences it as one stream of messages. Make sure the count is by entry and not by entry and clinician.

FHIR: if offers are mirrored to an external system through FHIR resources, the cap does not change what is exported. Offers that are not sent should not be exported as sent. Check the exporter reads the same status field that the cap uses so the two stay consistent.

## Testing

Unit tests for the config reader and the counting scope, as listed above. Job-level tests for selection: an entry under the cap is eligible, an entry at the cap is skipped, the next candidate is chosen, and all-capped leaves the slot open. A test for the day boundary using a clinic in a zone far from UTC, with an offer made shortly before and shortly after local midnight. A concurrency test is harder; at minimum write one that runs two promotions against the same entry in threads and asserts the cap holds, and mark it as potentially flaky if it needs to be.

For manual checks in staging, set the variable low, create a small waitlist, release several slots in a row, and watch that offers stop for the first entry and move to the others. Then wait for the local day to roll over, or move the clock in a test environment, and confirm the entry becomes eligible again.

## Rollout

Ship with the default of three, since that is the intended production value. Because the variable has a safe default, there is no need for a feature flag. Deploy to staging first, run the manual check, then production during a quiet window. Watch the skip log for the first few days. If the cap fires constantly, the likely cause is a ranking problem or a very short waitlist, not a wrong cap value, and that should be looked at before anyone raises the number.

Rollback is simple: deploy the previous release. The new index and status column are harmless to leave in place.

## Open questions

- Does a withdrawn-before-delivery offer count? See the section on what counts. Needs an answer from whoever owns notifications.
- Should staff be able to override the cap for a specific entry, for example a patient who is urgent? Probably yes eventually, but not in this change. Note it as a follow-up.
- Should the cap be settable per clinic instead of globally? The single environment variable is global for the app. Small clinics may want different values. Do not build per-clinic settings until someone asks for them.
- Is the surfaced skip message on the schedule view worth a design pass, or is the log enough for now? Ask the front-desk contact before spending time on UI.
