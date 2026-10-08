---
id: 01M241256Y67D88VQNTPEJSSZP
created: 2026-09-09T18:26-03:00
sources:
  - "doc: Capacity Report Q3"
---

# appointments-table growth and calendar query latency

The appointments-table reached 2.1 million rows in the third quarter, and the calendar query still held a 95th percentile of 12 ms over that period. So size has not yet hurt the front-desk calendar view. This note records where things stand so nobody has to re-derive it, and what to watch as the table keeps growing.

## What we know

- Row count: 2.1 million in the third quarter, in the appointments-table on MySQL.
- Calendar query latency: 95th percentile of 12 ms at that size.
- The calendar query is the one front-desk staff hit most. It reads a clinician's or room's appointments for a window of days, so it is the first thing that would feel slow if the table got out of hand.
- Nothing in the data says the current latency is at risk right now. The number is a snapshot, not a trend.

## Why it matters

ClinicSlotter is used by front-desk staff at small clinics. They open the calendar constantly while on the phone with patients, so a slow calendar is noticed immediately. Scheduling also has to respect clinician availability and room constraints, which means the slotting logic reads from the appointments-table on every attempt to book. Growth in the table affects both the calendar view and booking checks.

Most of the rows are likely old, finished appointments that nobody looks at in the calendar. The hot part of the table is the recent and upcoming window. As long as the calendar query stays bounded to that window and is served by the right index, total row count matters less than it seems. This is the reason 12 ms is plausible at 2.1 million rows. I have not confirmed this against the query plan in this session, so treat it as the working explanation, not a verified one.

## Things to watch

- Whether the 95th percentile of the calendar query drifts upward from 12 ms as the table grows past 2.1 million rows. Compare against this note rather than against a feeling.
- Background jobs run through Sidekiq, including work that touches appointments such as reminders and FHIR sync. Heavy scans there can compete with the calendar query for the database, so check them if latency moves.
- HL7 FHIR exports and imports read appointment data in bulk. Keep those off the hot path and away from the primary when possible.
- Heroku database plan limits (memory and connections) will bite before row count alone does, if the working set stops fitting in memory.

## Options if latency starts to climb

1. Check the query plan first and confirm the calendar query still uses the intended composite index on the time window and the clinician or room.
2. Archive or partition old appointments so the live table stays small. Archiving needs care because reporting and FHIR history may still want old rows.
3. Move bulk readers to a follower database.
4. Only after those, consider caching calendar windows.

## Open questions

- What is the growth rate per quarter? The 2.1 million figure is a single point, so projecting forward needs a second one.
- Is there a retention policy for old appointments, or do rows live forever?
- Does the 12 ms figure include cache hits from Rails, or is it measured at the database?

Update this note when the next quarter's row count and latency are known.
