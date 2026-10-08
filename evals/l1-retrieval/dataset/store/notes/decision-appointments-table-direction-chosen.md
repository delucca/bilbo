---
id: 01KSYD5ATC9GH9EQ8C61W8NR78
created: 2026-05-31T03:57-03:00
---

# appointments-table: general direction

This note records the general direction the team chose for appointments-table in ClinicSlotter. It is a direction, not a spec. It carries no thresholds or limits, because those change and live in code and config. If you are about to touch the table, read this before you add a column, an index or a background job.

The choice in one line: appointments-table stays the single place where a booked slot lives. It is a plain relational table in MySQL, owned by the Rails app. Everything else (waitlist logic, FHIR exchange, reminders, reporting) reads from it or asks the app to change it. Nothing else keeps its own copy of the truth about who is booked where and when.

We argued about this for a while. The alternatives were an event-sourced log with a projection, and a separate availability store that front-desk screens would read from. Both were rejected for the same reason: the clinics are small, the staff are not engineers, and when something looks wrong at the front desk someone has to be able to open one table and see what is going on. A table that answers "what is booked" directly beats a clever design that needs replaying.

## What the table is for

appointments-table holds the committed outcome of scheduling. A row means a patient has a slot with a clinician, usually in a room, for a span of time. It does not hold proposals, offers, or guesses. Those belong elsewhere and only become rows when someone, or a job acting on someone's behalf, commits them.

This matters because the scheduler has to respect clinician availability and room constraints. If tentative things leak into the table, every availability check has to ask whether a row is real, and that question spreads through the code. Keeping the table to committed bookings means the availability check can treat every row as a hard fact. We want to keep that property.

A few consequences follow from it.

- A row's meaning does not depend on which screen created it. Walk-in, phone booking, and a booking promoted from a waitlist all produce the same kind of row.
- Status changes (cancelled, completed, no-show and so on) are changes to the row, not new tables. We prefer a small set of states with clear meaning over many fine-grained states that staff will not use consistently.
- A cancelled booking stays in the table. Front-desk staff ask "what happened to that appointment" often, and deleting rows makes that question unanswerable. Availability queries must therefore always account for status. This is the most common place for bugs, so see the gotchas below.
- The table does not store derived display text. Names of clinicians and rooms come from their own records. We accept the join cost.

The table is not a general event log. If we need an audit trail of who changed what, that is a separate concern with its own storage. We do not want to stretch appointments-table to serve it, because every extra column that is only there for history makes the hot path heavier and the model harder to explain.

## Direction on integrity and conflicts

Double booking is the failure that hurts most in a clinic. Two patients in one room at the same time, or one clinician in two places, is visible within minutes and damages trust in the tool. The direction here is to defend against it in more than one layer, with the database as the last line.

In the application, the booking path checks clinician availability and room constraints before it writes. That check is for giving staff a useful, readable answer. It is not what guarantees correctness, because two staff members can submit at nearly the same moment and both pass the check.

For correctness we rely on the database. The preferred approach is to serialize competing writes for the same clinician or room by locking at the right granularity inside a transaction, and to re-check inside that lock. We chose this over optimistic retry loops as the default because the load per clinic is small, contention is rare, and a lock is easier to reason about than a retry policy. If a clinic ever grows into real contention, we revisit this, but we do not design for that case now.

We also want constraints in the schema wherever MySQL can express them cleanly. Not everything about overlapping time ranges can be expressed as a simple unique constraint, so the schema cannot carry the whole rule. The direction is: put into the schema what it can enforce, enforce the rest in a transaction with locking, and never rely on the UI alone. When someone proposes dropping a database constraint because "the app already checks", the answer is no.

Time handling needs a single convention. The table stores instants in one canonical zone, and conversion to the clinic's local time happens at the edges (display and input). Clinics can sit in different zones, and daylight changes have bitten scheduling tools before. Do not store local wall-clock times in the table. Do not compute durations from local times. If you find code that does, treat it as a bug to fix, not a pattern to copy.

Soft rules, such as preferring a particular room for a particular clinician, are not integrity rules. They belong in the scheduler's suggestion logic and must not be encoded as constraints on the table. A constraint that staff sometimes legitimately need to override is a constraint that will be worked around, and worked-around constraints are worse than none.

## Direction on schema change and background work

appointments-table is read constantly and written in short bursts. Changing it on a live Heroku deployment has to be boring. The direction is that schema changes to this table are done in small, reversible steps, each deployable on its own, with the application tolerant of both the old and new shape during the transition. Add the new thing, start writing it, backfill, switch reads, and only then remove the old thing, in separate releases.

Backfills run through Sidekiq, in batches, and must be safe to stop and restart. They must not hold long locks on the table. A backfill that blocks front-desk booking during opening hours is a production incident, even if it is "just a migration". Prefer to run heavy work outside clinic hours, and make the job pace itself so it yields to normal traffic.

Indexes are added deliberately. Each index should have a stated query it serves, written in the migration or the pull request. We do not add indexes speculatively, because each one slows writes and takes space, and the table is write-sensitive at the moment of booking. When a query is slow, look at the query and the existing indexes before adding another.

Sidekiq jobs that touch the table must be idempotent. Jobs can be retried or run twice, and a job that books or releases a slot has to produce the same end state if it runs again. The usual technique is to have the job re-read the current row and decide from that, rather than trusting arguments that may be stale by the time it runs. Jobs pass identifiers, not copies of row data.

The related waitlist work follows these same rules. How the promoter turns an opening into an offer, and when an accepted offer becomes a committed row, is covered in [[waitlist-promoter-offers-entry]]. The short version for this note: an offer is not a row. Only acceptance, through the normal booking path, creates one. Do not add a shortcut that inserts directly into appointments-table from the promoter.

External exchange through FHIR is handled as a translation layer. The table is not shaped around FHIR resources. We map to and from FHIR at the boundary, so that a change in what an external system wants does not force a change in our core table. Inbound data is validated and converted into the normal booking path rather than written straight in. Outbound representations are built from the rows on demand or by a job, and failures to send do not roll back a booking that is already committed. The booking is the fact; the message is a report of it.

A small sketch of the intended flow, using only the parts we already have:

```
Rails app -> MySQL (appointments-table)   # source of truth for bookings
Sidekiq   -> re-reads rows, then acts     # reminders, offers, cleanup
FHIR      -> built at the edge            # never the shape of the table
```

## Gotchas and things to keep doing

These are the habits that keep the direction intact. Most come from places where the code could easily drift.

- Always filter on status when asking whether a slot is free. A cancelled row must not block a slot. Put this in one shared scope or query object and reuse it, so nobody re-derives it by hand.
- Do not delete rows to cancel. Change the status. If a real deletion is ever needed, for example for a data request, it goes through a reviewed, explicit path and not through ordinary screens.
- Keep the booking path singular. Every way of creating or moving a booking goes through the same service, which does the check, takes the lock and writes. If a new feature needs a new way to book, extend that service, do not write around it.
- Be careful with bulk updates. Mass status changes, such as closing a clinician's day, skip callbacks and validations if written as raw updates. Decide on purpose whether that is acceptable, and say so in the code.
- Keep transactions short. Do not call out to FHIR endpoints or other slow services while holding a lock on bookings. Commit first, then enqueue the follow-up work.
- Enqueue background work after commit, not inside the transaction. A job that runs before the row is visible will fail in confusing ways and look like a flaky test.
- Treat reads for the schedule screen as a performance-sensitive path. It is what staff look at all day. Load what the screen needs in few queries and avoid per-row lookups. Watch for hidden per-row queries when a view touches clinician or room names.
- Test the unpleasant cases: two submissions at the same moment, cancelled rows next to live ones, bookings that straddle a change of local time, and retried jobs. The happy path is rarely where this table breaks.

## What we decided not to do

We did not split appointments into separate tables per clinic or per kind of visit. It would complicate every cross-cutting query and gain little at our size. Clinic scoping is done with a foreign key and a consistent scope in the models, and tenancy checks live in the application layer in one place.

We did not move availability into a cache that the UI trusts. Caches are fine for speeding up display, but a cached answer must never be what decides whether a booking is allowed. The decision is made against the table, inside the lock.

We did not put business rules in database triggers. They are invisible to most of the team, hard to test alongside the Rails code, and awkward to deploy on Heroku. Constraints that the schema can express natively are fine. Logic that needs conditionals and context stays in the app.

We did not make the table the home for every note, flag or preference that staff want attached to a visit. Free-form additions pile up there quickly. If a piece of information has its own lifecycle, it gets its own record that points at the appointment.

## When to revisit

This direction fits small clinics with modest concurrency, and it should be revisited if that stops being true. Signals worth acting on: lock waits showing up in real use, staff reporting slow schedule screens at busy times, or a need for history that the status field cannot answer. None of these is a reason to abandon the single-table, database-backed approach by itself. Try the smaller fixes first (better queries, finer lock scope, a separate history record) and only reopen the larger question if those fail.

If you change something that contradicts this note, update the note in the same change, and say why. A stale decision note is worse than none, because the next person will trust it.
