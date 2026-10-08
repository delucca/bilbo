---
id: 01M391WC216Z3AYT50Z9E1R9VM
created: 2026-09-24T03:33-03:00
---

# appointments-table archive plan

This note replaces the earlier note "appointments table archive rows". The new rule: archive appointments-table rows older than 36 months. The earlier plan used a 24 month retention window; drop that one. Anything that still mentions the shorter window is out of date and should be changed to match this note.

## What changed

The old plan archived rows from appointments-table once they were two years old. That was too aggressive for small clinics. Front-desk staff look back at past visits more often than we assumed, for example when a patient returns after a long gap, or when a clinic needs to reconcile a billing question. With the shorter window those rows would already be in the archive and slower to reach. So the cutoff moves out to 36 months. Rows younger than that stay in the live table.

The rule is about age only. It does not depend on status, clinician or room. Cancelled and no-show rows follow the same cutoff as completed ones, so the job stays simple and nobody has to remember special cases.

## Plan

Do the work in small steps so each one can be checked on its own.

1. Confirm what the live appointments-table holds today, and how many rows fall between the old cutoff and the new one. Those rows were slated for archiving under the old plan and now should stay put. If any were already moved, they need to be put back.
2. Change the retention setting wherever the old value is written down: the archive job configuration, any scheduled task definitions, and the docs. Keep a single source for the value so it is not repeated in three places.
3. Run the archive as a Sidekiq job in batches, off peak, so it does not compete with front-desk use during clinic hours. Small batches, with a pause between them, keep MySQL load and lock time low.
4. Move rows into the archive store first, verify the copy, and only then delete from appointments-table. Never delete first.
5. Log how many rows each run moved, so a bad run is easy to spot.

## Things to watch

Scheduling queries must not break. The slot search looks at clinician availability and room constraints, and it should only need current and future rows. Check that none of the queries or reports reach back past 36 months and silently return less than before.

FHIR export is the other risk. Appointment resources that are sent out or looked up by id may point at rows that are now archived. Decide whether lookups for archived rows return from the archive or return not found, and make that explicit rather than accidental.

Foreign keys and indexes: anything that references appointments-table rows needs a check before deletion, so archiving does not leave dangling references or fail on constraints.

Heroku adds a practical limit. Long-running work can be cut off by dyno restarts, so the job has to be safe to rerun. A rerun after a restart should pick up where it stopped and never copy a row twice.

## Open questions

- Where the archive lives: a separate table in the same MySQL database, or a different store. This affects how restores and FHIR lookups work.
- Whether archived rows ever get purged, and on what schedule. This note does not set one.
- Who signs off on retention for each clinic, since small clinics may have their own rules about how long records stay readily available.

## Status

Not started. The only decision made so far is the cutoff: 36 months, replacing the earlier 24 month plan. The steps above are the intended order of work.
