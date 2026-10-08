---
id: 01KQY1SHNS27VNQ2D0YE4A82PQ
created: 2026-05-06T04:07-03:00
---

# export-worker fails on very large events with context deadline exceeded

Exporting a very large event makes export-worker fail with `context deadline exceeded`. The cause is the read query: on a big event it runs past its timeout, and the database call is cancelled before any rows come back. The rule that follows is that exports must page by event_id and created_at. Never let export-worker read a whole event in one query.

## Symptom

A producer asks for an export of a big event, such as a long town hall with a huge number of poll votes and Q&A entries. The job starts, sits for a while, then dies with `context deadline exceeded`. Small and medium events export fine, which is why this slips through testing. The failure depends on event size, not on the time of day or on a particular producer.

The error text comes from the query timeout, not from the export logic. Nothing is wrong with the file writing or the download link. Do not go looking in the Next.js front end or in the WebSocket layer for this one.

## Cause

export-worker used to issue a single read against CockroachDB for everything belonging to an event. For a very large event that read takes longer than the timeout set on the query. When the deadline passes, the context is cancelled and the worker surfaces `context deadline exceeded`. Raising the timeout only moves the point where it breaks, and it ties up a database connection for longer while live events are running, so it is not the fix.

## Rule: page by event_id and created_at

Every read in export-worker must be paged, with event_id and created_at as the paging keys. In practice:

- Filter on the event_id being exported.
- Order by created_at, with a tiebreaker so rows with the same timestamp are not skipped or repeated.
- Fetch a bounded chunk, write it out, then ask for the next chunk starting after the last created_at seen (keyset paging, not offset paging).
- Each page is its own query with its own timeout, so no single read has to cover the whole event.

Offset paging gets slower the deeper it goes and can hit the same timeout on late pages, so keep to keyset paging on those two columns. Make sure the index serving the query matches event_id then created_at, otherwise each page may still scan too much.

## Checking a change

When touching the export path, test against a very large event, not a small fixture. A passing run on a small event proves nothing about this bug. Confirm that no `context deadline exceeded` shows up in the worker logs, and that the row count in the export matches what is in the database for that event. Also check that rows written while the event is still live do not cause duplicates or gaps between pages; paging on created_at keeps that stable as long as the tiebreaker is in place.

## Where to look

The export query and its timeout live in the export-worker code. If this error returns after paging was added, first check that every query in the worker pages, including any count or summary query run before the main export, since one unpaged read is enough to bring the failure back.
