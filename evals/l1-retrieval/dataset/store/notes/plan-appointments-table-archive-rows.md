---
id: 01M1B0DZ84QD4CM430J2KJ17Q5
created: 2026-08-31T01:14-03:00
---

# Plan: archive old appointments-table rows

Plan to move old rows out of appointments-table so the live table stays small and the front-desk scheduling views stay fast. The rule: any appointments-table row older than 24 months gets archived into the table appointments_archive. Nothing is written yet; this is the order of work.

## Why

appointments-table grows with every booking, reschedule and cancellation. Slot search and room conflict checks read it constantly, and most of what they scan is history nobody at the front desk looks at. Moving old rows out keeps the hot table small without losing the data.

## Scope

- Source: appointments-table.
- Destination: appointments_archive, same columns as the source so a row can be moved as is.
- Cutoff: rows older than 24 months. Younger rows stay put.
- Not in scope: changing how new appointments are created, or changing FHIR exports for current appointments.

## Steps

1. Add a migration that creates appointments_archive with the same schema as appointments-table, plus an archived_at timestamp.
2. Write a Sidekiq job that selects rows past the cutoff in small batches, copies them into appointments_archive, then deletes them from appointments-table inside one transaction per batch.
3. Schedule the job to run off-hours so it does not compete with clinic opening times.
4. Make the job safe to re-run: skip rows already present in the archive, so a crash mid-run does not duplicate anything.
5. Run it on a staging copy first and compare row counts before and after.

## Sketch

```ruby
# archive appointments-table rows older than 24 months
cutoff = 24.months.ago
# copy into appointments_archive, then delete from the source, per batch
```

## Risks and open questions

- Foreign keys: anything that references appointments-table rows (rooms, clinician availability, reminders) must be checked before deleting. Either those references move too or they must tolerate missing rows.
- Reports and FHIR resources that read old appointments will need to look in appointments_archive as well, or be told history is no longer there. Needs a decision from whoever owns reporting.
- Retention rules for clinical data may require keeping the archive for a set time. Not confirmed yet.
- MySQL deletes on a large table can lock for a while, so batch size matters. Pick it from the staging run.

## Rollback

Keep the copy step separate from the delete step at first. If something looks wrong, rows can be copied back from appointments_archive into appointments-table, since the schemas match. Do not drop the archive table until the job has run cleanly for a while.

## Next

Confirm the foreign key and reporting questions, then write the migration and the job.
