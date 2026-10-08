---
id: 01KH36004MNG7KFVF9K6AYEG10
created: 2026-02-10T04:07-03:00
---

# appointments-table: where the pieces live

This is a map of the appointments-table component in ClinicSlotter, not a spec. It says where to look, roughly, and what usually lives next to what. Details change often, so check the code before you trust any of it. Nothing here records a decision or a limit with a value in it. If you need one of those, read the code or ask whoever owns that area.

The appointments-table is the MySQL table that holds every booked, held, moved or cancelled visit. Front-desk staff never see it as a table. They see a calendar grid, a search box and a few forms. Everything they do ends up as a write to this table, or as a read from it filtered by clinician, room and time window. Because of that, a lot of code touches it, and the pieces are spread over the Rails app, the Sidekiq workers, the FHIR side and the Heroku setup. This note walks through those one by one.

Related note: [[fhir-gateway-must-time]]. Read it before changing anything on the FHIR edge, because that is where the table meets outside systems and the timing there bites.

## Schema and migrations

The schema lives in the usual Rails places: the migrations directory under the database folder, and the schema dump next to it. The dump is the quickest way to see the current shape of the appointments-table. The migrations are the way to see how it got that shape, and a few of them carry comments explaining why an index exists. Read those comments before you drop or reorder an index. Several of the indexes serve the calendar view and the conflict check, and removing one looks harmless until a busy clinic loads its week view.

What the table holds, in general terms:

- A link to the patient record, a link to the clinician, and a link to the room, when a room is needed at all.
- A start and an end, stored in a single time standard, with the clinic's local zone resolved at the edges rather than in the column.
- A status field that says whether the slot is booked, held, checked in, completed, cancelled or marked as a no-show. It is an enum on the model side and a plain column in MySQL.
- A visit type, which decides default length and which rooms are eligible.
- Bookkeeping: who created it, who last changed it, timestamps, and a soft-delete or cancellation marker depending on how that part of the code evolved.
- Fields that tie the row to an external FHIR resource, so the sync can find its counterpart.
- Free-text notes from the desk. These are short and not searched.

MySQL specifics worth remembering. The engine is MySQL, not Postgres, so there is no exclusion constraint that could stop two overlapping rows from existing. Overlap protection is done in application code and with locking, described below. Collation and time precision come from the database config and from the migration defaults; if a comparison between two times behaves oddly, check precision on the column first, then the Rails side.

Schema changes go through the normal Rails migration flow. On Heroku the release phase runs them. Large table alterations on a live clinic system are a risk because the table is hot during working hours. When a change needs a long rewrite, it has been done in stages in the past: add the column, backfill in a Sidekiq job, switch reads, then clean up. Look for earlier migrations that did this and copy their shape instead of inventing a new one.

## Models and the Rails layer

The main model for the table sits in the models directory of the Rails app and carries most of the rules. It is the first file to open. Typical contents:

- Associations to patient, clinician, room, and to the visit type.
- The status enum and the scopes built on it, such as active, upcoming, for a given day, for a given clinician.
- Validations for required links, end after start, and the visit type being allowed for the chosen room.
- Callbacks. There are some, and they are a source of surprises. Some enqueue background work after commit, some write audit rows, some touch the parent clinician record so caches expire. If a save behaves strangely in a console or a test, read the callback list at the top of the model before anything else.

Next to the model there are usually small plain Ruby objects, often in a services or a domain folder, that wrap the real operations: book, reschedule, cancel, check in, mark no-show. The controllers call these objects rather than writing to the model directly. That is the intended path. If you find a controller or a rake task that updates the table with a raw update, treat it as old code, not as a pattern.

The overlap and availability logic is its own cluster. It takes a clinician, an optional room and a time window, and answers whether the slot is free. It reads the appointments-table, the clinician availability data (working hours, blocks, leave) and the room data (opening, maintenance). Look for a folder or namespace with a name about scheduling, slotting or availability. The finder that proposes slots for the desk is built on the same pieces, so a change to the conflict rule changes what the desk is offered.

Locking is part of that cluster. To avoid two desk users booking the same clinician at the same moment, the booking path takes a lock on something stable, usually the clinician row or a per-clinician scope, inside a transaction, then re-checks for overlap, then writes. If you add a new write path, it must go through the same lock, or you create a way to double-book. This is the most important rule about the table, and it is enforced by convention, not by the database.

Serializers and presenters live in their own place. The calendar grid reads from a presenter that shapes rows into blocks with colours and labels. The JSON endpoints used by the front end use serializers. When a field is added to the table, it is not exposed anywhere until one of these is updated, so a "missing field" report is usually a serializer problem.

Controllers and routes: the desk-facing controllers for appointments are in the controllers directory, with a separate namespace for the API used by the browser front end and another for anything machine-to-machine. Authorisation is checked per clinic, since each clinic only sees its own rows. Multi-tenant scoping is applied through a default scope or a controller-level finder; check which one before writing a new query, because a missed scope would show another clinic's data. Reports and exports that bypass controllers need the same scoping and have been a place where it was forgotten.

## Background jobs

Sidekiq runs the asynchronous parts. The worker classes live in the workers or jobs directory, and their queue names are listed in the Sidekiq config. The ones that touch the appointments-table, in general terms:

- Reminder jobs. They scan upcoming rows and send messages. They read; they should not change the booking.
- Sync jobs to and from FHIR. They push a changed appointment out, and they pull changes in. These write to the table, but only to a limited set of fields. More on this in the next section.
- Housekeeping jobs. They close out old holds that were never confirmed, flip past booked rows to completed or no-show according to the rules, and clear stale temporary data.
- Backfill jobs from past migrations. Some may still be in the codebase long after use. They are safe to leave alone and worth deleting only after confirming nothing enqueues them.
- Audit and notification jobs triggered by model callbacks.

Rules of thumb for these. Jobs should be idempotent, since Sidekiq retries. A job that is given an appointment reference should reload the row and check its current status before doing anything, because the desk may have cancelled or moved it between enqueue and run. Jobs are enqueued after commit, never inside the transaction, otherwise the worker can run before the row is visible. If you see a job that takes whole objects as arguments, it is wrong; pass a reference.

The scheduler for periodic jobs is configured in a separate file from the worker classes. If a housekeeping job seems not to run, check that file and the Heroku process types before suspecting the job code. Queue priority and concurrency are set in config and on the dyno side, and they interact with the database connection pool; if workers start timing out on connections, look at the pool size against the thread count.

## FHIR side

The appointments-table is mirrored outward as FHIR Appointment resources, and, to a smaller extent, inward. The code for this sits in its own area, usually a folder named for FHIR or the gateway, separate from the core scheduling code. It contains:

- Mappers that turn a table row into a FHIR resource and back. The status mapping between the local enum and the FHIR status codes is here, and it is not one-to-one; some local states collapse into a single FHIR state. Read the mapper before assuming a round trip is lossless.
- A client wrapper around the HTTP calls to the external FHIR server, with its own timeout and retry settings. That is the part the related note is about, so go to [[fhir-gateway-must-time]] for the reasoning and keep it in mind when you touch any call that waits on a remote.
- Inbound handlers, for the cases where an outside system creates or edits an appointment. These must go through the same booking objects as the desk, including the lock and the overlap check. If an inbound path writes straight to the table, it can create double bookings, and that is a bug to fix, not to copy.
- Reference lookups for patients, practitioners and locations, which map external identifiers onto local rows. The link fields on the appointments-table depend on these lookups succeeding.

Things that go wrong here, in general. A remote system being slow makes a Sidekiq worker hold a connection and a thread for a long time. A remote system returning a resource for a clinician or room the local database doesn't know about leaves the row half-linked. Two systems editing the same appointment at once produces a last-writer-wins result unless the sync compares a version or a modified marker; check how the current code does it before changing the sync order. None of this is solved by retrying harder.

Tests for the FHIR side use recorded or stubbed responses kept near the specs. When the mapper changes, update the fixtures with it.

## Tests, fixtures and local setup

The specs for the table are spread by layer. Model specs cover validations, scopes and enum behaviour. Service specs cover the book, move and cancel operations, including the overlap cases and the locking behaviour. Request specs cover the desk and API endpoints with clinic scoping. Worker specs cover idempotence and the reload-and-check behaviour. FHIR specs cover mapping and the client wrapper with stubs. Look in the spec directory in the same structure as the app directory and you will find them.

Factories for appointments, clinicians, rooms and patients are in the factories folder. The appointment factory has traits for the common states. If you need a booking in a tricky state, prefer a trait over setting columns by hand, and add a trait if none fits. Be careful with time in tests: the code treats the clinic zone carefully and the tests should freeze time rather than rely on the machine clock. A test that passes in the morning and fails in the evening is almost always a zone or clock problem.

Local setup uses a local MySQL and a local Redis for Sidekiq. The seeds create a small sample clinic with a few clinicians and rooms and some appointments around the current day. They are handy for looking at the calendar grid. Do not copy production data to a laptop; the table holds patient-linked rows and that is not allowed outside the production environment. If you need a realistic dataset, extend the seeds or the factories instead.

When a spec for this table is flaky, the usual causes are, in order: time not frozen, shared state between examples in the database cleaning strategy, and job queues not being drained or cleared between examples. Check those before adding retries.

## Deployment, operations and who to ask

The app runs on Heroku. Web and worker processes are separate process types in the process file, with the release phase running migrations. Config values, including database and Redis addresses and the FHIR credentials, are in Heroku config vars, not in the repo. Do not paste them into notes, tickets or chat.

For the appointments-table in production, operational concerns in general terms:

- Backups and restore are handled by the Heroku database add-on and its schedule. A restore to a point in time affects every clinic at once, so it is a team decision, not an individual one.
- Slow queries on the table usually come from the calendar view or the search box. The first thing to check is whether the query used the intended index and whether it was scoped by clinic. Look at the query log or the add-on's monitoring before changing code.
- Lock waits and deadlocks show up when many desk users book against the same clinician, or when a long sync job holds rows. They are rare but they are the sign that something took a lock outside the usual path.
- A growing table slows the housekeeping jobs. Archiving of old rows has been discussed and any such change needs care because reports and FHIR history depend on old rows still being there.
- Rolling back a deploy does not roll back a migration. A migration that has run stays run, so write changes to the table to be safe with both old and new code during a release.

Logs go to the Heroku log stream and to whatever aggregator is attached. Search by appointment reference rather than by patient to keep personal data out of searches. Audit rows, where they exist, are the better source for who changed what.

Checklist for changing something around the table, in the order I would do it:

- Find the operation in the services layer and read it end to end, including its callbacks on the model.
- Check whether the change affects the overlap rule or the lock. If so, read the conflict check and its specs first.
- Check the serializer, the presenter and the FHIR mapper for the field or state you are changing.
- Check the workers that read or write the same rows, and whether they reload before acting.
- Write the migration so it works with old code running, and plan a backfill job if existing rows need new values.
- Add or update specs on each layer you touched, with time frozen.
- Read [[fhir-gateway-must-time]] if any step waits on a remote system.

Where this note is weak: it does not list the exact files, since they move, and it does not describe the clinic-specific rules some clinics have for visit types and rooms. Those live in configuration data and in a few clinic-specific branches in the scheduling code, and they are the first thing to look for when a clinic reports behaviour that the generic rules don't explain. Update this note when a piece moves, and delete lines that are no longer true instead of leaving them.
