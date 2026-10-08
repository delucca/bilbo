---
id: 01KDZ7TRNC0GQ30HPZ7QB9BY0W
created: 2026-01-02T08:35-03:00
sources:
  - "code: db/schema.rb"
---

# room-constraints design

room-constraints holds the per-room, per-weekday rules that the scheduler checks before it offers a slot to front-desk staff. One row says what a given room allows on a given day of the week: when it is usable, and what limits apply. The scheduler reads these rows next to clinician availability and drops any slot whose room does not fit. This note covers the table shape, the uniqueness rule, how the rows are read, and the things to watch when changing it.

## Table shape

Each row belongs to one room and one weekday. The row carries the window in which the room can be booked on that day and any limits that narrow it, such as a closed flag or restrictions on the kind of appointment the room can host. Times are stored in the clinic's local zone, not UTC, because front-desk staff think in clinic time and a room's opening hours do not shift with the server. The table lives in MySQL and is managed through normal Rails migrations.

The key rule is uniqueness. The room-constraints table carries the unique index `idx_room_constraints_room_day` on the columns room_id and weekday. So a room has at most one constraint row per weekday. If a room needs two separate windows in one day, that is not modelled by two rows; the single row has to describe it, or the shape of the table has to change first. Do not try to get around the index by inserting near-duplicate rows.

## Why one row per room and day

The scheduler needs a single answer to "is this room usable at this time on this weekday". With one row, that is a lookup, not a merge of several rules that might contradict each other. The unique index enforces this at the database level, so a bug in the app or a race between two requests cannot leave two competing rows for the same room and day. Application validations exist too, but the index is what actually holds when two writers collide.

## How it is read

The slot search loads the constraints for the rooms in play once per request and keeps them in memory, keyed by room and weekday. It does not query per candidate slot. Slot generation runs in Sidekiq jobs for bulk work, such as filling a week, and the same lookup is used there. A room with no row for a weekday is treated as not constrained by this table for that day; check the scheduler code before relying on that, since it is a default and not something the schema enforces.

Where room data crosses into HL7 FHIR resources, the room maps to a location, but the constraint rows are internal. They are not exported as FHIR resources, so changing them does not alter anything sent to outside systems.

## Writing and changing rows

Writes should be upserts against the room and weekday pair, so that saving the same room and day twice updates the row and does not raise a uniqueness error. When a duplicate error does appear, it almost always means a caller built a new record when it should have found the existing one.

When changing the table:

- Keep the index in every migration that rebuilds or renames the table. Losing it silently allows duplicates.
- Clean up duplicate rows before adding any new unique column to the index, otherwise the migration fails halfway on production data.
- On Heroku, run migrations in the release phase and avoid long locking changes during clinic hours.
- Add a test that inserting a second row for the same room and weekday fails.

## Open points

Split-day windows are the main gap. Clinics that close a room over lunch currently have to express that some other way, and it is worth deciding whether to allow several windows inside the one row or to widen the key. Either choice touches the index, so settle it before building anything on top.

Cache invalidation is the other point. If constraints are cached longer than one request, an edit by front-desk staff must clear the cache, or the scheduler will keep offering slots in a room that was just closed.
