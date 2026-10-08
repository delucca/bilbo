---
id: 01KZE07VA76R4X5TD5FH7XH5JT
created: 2026-08-07T08:37-03:00
---

# Weekly recap: appointments-table

Quick recap of the week on appointments-table. Most of the time went into reading how slots get written and read, then tightening a few things that were loose. Nothing here is final; it is what I would want to know on picking this up again.

## Where things stand

The appointments-table is still the central place where booked slots live, and almost every screen the front desk uses reads from it. It behaves, but it is showing its age in the queries that filter by clinician and room. This week was mostly cleanup and investigation, not new features.

## What I worked on

Mainly three things: reviewing the indexes against the queries the scheduler actually runs, going through the background jobs that touch the table, and checking how FHIR resources map to rows. I also spent some time on tests that were flaky around overlapping slots.

## Index review

I compared the existing indexes with the real query shapes coming out of the scheduling code. A couple of them look redundant, and one common lookup seems to fall back to a wider scan than it should. I have not changed anything in production. The next step is to confirm with a query plan on a copy of realistic data before touching the schema.

## Clinician availability lookups

The availability check joins against the appointments-table to find conflicts. It works, but the join is read-heavy at busy times of day. I sketched a narrower query that only asks for the time window in question. It needs a proper test with edge cases such as appointments that end exactly when another begins.

## Room constraints

Room conflicts are enforced partly in the app and partly by what is stored. I found one path where the room check runs after the write attempt instead of before. It has not caused visible trouble, but it makes error handling awkward. I noted it and left it alone for now.

## Sidekiq jobs

Several jobs update rows in the appointments-table: reminders, status sync, and cleanup of stale holds. They mostly run fine. The status sync job can retry in a way that rewrites the same row more than once. That is harmless today but noisy in logs. I want to make it idempotent in a clearer way.

## Locking and concurrency

Two front-desk users booking the same slot at nearly the same moment is the case I keep coming back to. The current protection depends on a check followed by an insert. I reread that code and think a database-level guard is the safer route. I did not implement it yet because it affects how errors surface to the user.

## FHIR mapping

Appointment resources are built from rows and sometimes parsed back into rows. Most fields map cleanly. A few optional fields get dropped silently on the way in. I listed them for a later pass; they matter for clinics that exchange data with outside systems.

## Time zones

Times are stored in one consistent form, but display logic in a couple of places converts more than once. This is a likely source of the odd off-by-an-hour reports from clinics. I have a suspect but no reproduction yet.

## Data quality

I looked over rows that have missing or odd status values. They are rare and seem to come from older imports. A one-off cleanup is probably fine, but it should be reviewed by someone who knows the clinic data before running anything.

## Tests

Fixed a couple of flaky specs that depended on the current time and on record ordering. Added a few cases for back-to-back appointments. Coverage on cancellation and rescheduling is still thin and should be next.

## Migrations

No migrations went out this week. I drafted notes on what a safe one would look like given MySQL and the need to avoid long locks on a live table at Heroku. Any index change should be done in a way that does not block the front desk during clinic hours.

## Performance observations

Slow spots are concentrated around the daily calendar view and the search by patient. Nothing alarming, but they would get worse as clinics grow. I did not profile deeply; this is a read of the code plus what the logs suggested.

## Risks

The biggest risk is concurrent booking. The second is the quiet loss of FHIR fields. Third is any schema change done without testing against realistic volumes.

## Open questions

Should the room check move earlier in the booking flow for all paths? Are the older imported rows still needed in their current shape? Does anyone depend on the redundant indexes through a report I have not seen?

## Next week

Run query plans against realistic data. Decide on the database-level guard for double booking. Make the status sync job safer to retry. Add cancellation and reschedule tests. Follow up on the time zone suspect.

## Things to avoid

Do not run cleanup on stale rows before someone reviews the selection. Do not change indexes directly on the live table without a plan for locking. Do not assume the FHIR round trip is lossless.

## Handoff notes

If someone else takes this over, start with the booking path and the availability query, since most other issues connect to those. Keep changes small and reviewable. Leave a short note here when something moves from suspected to confirmed.
