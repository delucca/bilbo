---
id: 01K3MB66ZTS6GH2YMEX16Y7FA7
created: 2025-08-26T20:25-03:00
---

# appointments-table: starts_at stored as UTC DATETIME

Decision: appointments-table stores `starts_at` as a UTC `DATETIME`, not a TIMESTAMP. The reason is the year 2038 limit. MySQL TIMESTAMP columns cannot hold moments past that point, and clinics book appointments far enough ahead, and keep recurring series long enough, that we don't want a column type with a hard ceiling. `DATETIME` has no such ceiling for any date we will realistically schedule.

Short name: `apptbk` is short for `appointments-table`. You will see `apptbk` in chat, in branch names, and in some Sidekiq job names. It is the same thing as `appointments-table`. In this note and in new docs, write `appointments-table` in full.

## What was decided

The value in `starts_at` is always UTC. The column type does not carry a time zone, so the convention is the only thing that makes it UTC. Whatever writes to appointments-table has to convert to UTC before saving, and whatever reads from it has to convert to the clinic's zone for display. A `DATETIME` holds the wall-clock digits it was given and nothing else. A TIMESTAMP would have converted values through the session time zone behind our back. We lose that convenience and gain a column that behaves the same no matter how the connection is configured.

In short:

- `starts_at` is `DATETIME`, in UTC, by convention.
- Rails owns the conversion. Application code works with time-zone-aware values and the database layer stores UTC.
- The database server and connection time zone settings should not matter for this column. If a fix ever depends on them, something is wrong.
- Other time columns in appointments-table follow the same rule unless a later decision says otherwise.

## Why not TIMESTAMP

TIMESTAMP in MySQL is stored as a count of seconds since the epoch in a signed 32-bit field. That is where the year 2038 limit comes from. Writes past the limit fail or are rejected, depending on SQL mode. For a scheduler that would show up as a booking error for a far-future date, probably reported by front-desk staff as the app refusing a perfectly normal appointment. Nobody wants to debug that late, and migrating a large live appointments-table to a different column type is a bigger job than choosing the right type now.

TIMESTAMP does have real advantages, and they were weighed:

- It converts to and from the session time zone automatically. For us this is a hazard more than a help, since the Rails app, Sidekiq workers, console sessions and ad hoc SQL can all have different session settings.
- It is a little smaller on disk. At the size of a small-clinic dataset this is irrelevant.
- It has automatic initialization and update behavior. We don't rely on that for `starts_at`, because an appointment's start time is business data, not a record-keeping stamp.

None of these outweigh a hard ceiling on the one field the product is about.

## Consequences for code

Rails side. ActiveRecord reads and writes `starts_at` as a time value in UTC when the app's default is UTC. Keep the app's default time zone and the ActiveRecord default timezone both on UTC, and convert for display at the edge, in views, serializers and mailers. Do not store local times in `starts_at` and then try to remember which clinic's zone they were in. Each clinic has its own zone, and that belongs on the clinic record, not baked into appointment rows.

Queries. Anything that asks for "today's appointments" for a clinic must compute the clinic's local day boundaries first, convert them to UTC, and then query `starts_at` with a range. Do not wrap `starts_at` in a date function in the WHERE clause. Besides getting the zone wrong, it defeats the index. This is the most likely place for a bug, especially around daylight saving changes, when a local day is longer or shorter than usual.

Clinician and room availability. The conflict checks that respect clinician availability and room constraints compare `starts_at` against availability windows. Those windows need to be in UTC too, or be converted the same way, before they are compared. A mismatch here would produce double bookings or phantom free slots, so any new availability feature should be tested near a daylight saving change.

Background jobs. Sidekiq jobs that touch appointments-table (reminders, rescheduling sweeps, nightly cleanups) should pass appointment ids, not time values, and load the record in the job. If a job must carry a time, pass it in an unambiguous form such as ISO 8601 with an explicit offset. Do not rely on the worker's local zone.

## FHIR and external data

HL7 FHIR Appointment resources carry their start as an instant with an offset. On import, convert to UTC before writing to `starts_at`. On export, emit the instant with an explicit offset, which UTC satisfies. Some FHIR data arrives as a date with no time or with a partial time. Do not guess a time and store it as if it were exact. Reject it or handle it in a separate step. The `DATETIME` column cannot tell us a value was guessed.

## Operational notes

Heroku hosts the app and the MySQL add-on. Neither changes this decision, but check that the database add-on's default time zone is not being relied on by any raw SQL or reporting script. Reporting queries that people run by hand against appointments-table should treat `starts_at` as UTC and convert explicitly. Put a comment at the top of any saved report that says so.

If you add a column to appointments-table that stores a moment in time, use `DATETIME` and UTC for it as well, for the same reason. The only exception is a pure bookkeeping stamp where the limit cannot matter in practice, and even then consistency is usually the better argument.

## Open points

- Whether to add a database-level check or a model-level validation that rejects obviously non-UTC input. For now the convention is enforced by code review and by the conversion at the Rails edge.
- Whether older rows from before this decision, if any exist in some environments, need an audit. If a TIMESTAMP column turns up anywhere on appointments-table, treat it as a bug against this decision and raise it rather than quietly working around it.
- Reporting tools outside Rails. Anyone connecting directly should be told `starts_at` is UTC `DATETIME`, since the schema alone will not say so.

If you revisit this, the question to ask is whether any real appointment could fall beyond the 2038 limit. For a scheduler that supports recurring series and long-range booking, assume yes.
