---
id: 01KPE4ACC3HBSW1FGGY89Q8PQY
created: 2026-04-17T13:27-03:00
sources:
  - "code: db/schema.rb"
---

# appointments-table design

The appointments-table is the core store for ClinicSlotter. Every booked outpatient visit is one row, and the front desk reads it all day to draw the calendar. The main read is "what does this clinician have on this day or week", so the table is shaped around that. It has a composite index, `idx_appt_clinician_start`, on clinician_id and starts_at. The index exists to serve calendar lookups. If you change how the calendar queries, check them against this index first.

## Why this index

Front-desk staff open a calendar view per clinician. The query filters by one clinician and a time window, then orders by start time. A composite index with clinician_id first and starts_at second fits that exactly: MySQL jumps to the clinician's rows and walks them in time order, with no filesort and no scan of other clinicians' rows.

The column order matters. clinician_id is an equality filter and starts_at is a range, so equality goes first. Flipping the order would make the index much less useful for the calendar, because a range on starts_at across all clinicians would come first.

## What it also helps

The same index covers the conflict check at booking time. Before a slot is saved we look for other appointments for that clinician that overlap the requested window. That is the same clinician plus time-range shape, so it uses the same index.

It does not help room lookups. Room constraints are checked by a separate path and are not served by `idx_appt_clinician_start`. If room checks get slow, that needs its own index and its own note, not a change to this one.

## Working with it in Rails

The index is declared in a normal Rails migration against the MySQL database. Keep it in the schema file so fresh Heroku review apps and local setups get it.

```ruby
add_index :appointments,
          [:clinician_id, :starts_at],
          name: "idx_appt_clinician_start"
```

Notes for later work:

- Scope calendar queries through a clinician first (`where(clinician_id: ...)` then the time range). Queries that skip clinician_id will not use the index well.
- Sidekiq jobs that sync appointments out to HL7 FHIR resources should read through the same clinician and time filter when they batch by clinician, so they do not do full table scans in the background.
- Adding columns to the table is cheap. Adding or rebuilding indexes on a large table on Heroku should be done off-peak, since clinics use the app during working hours.

## Open questions

- Whether cancelled appointments should stay in this index or move to a partial approach. MySQL has no partial indexes, so the options are a status column in the index or archiving old rows. Not decided.
- Whether a second index is needed once room-based views get heavier use. Wait for real slow-query evidence before adding one.
- Index size grows with appointment history. If it becomes a problem, archiving old rows is the first thing to try, before changing the index definition.
