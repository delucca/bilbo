---
id: 01K754JXTQATCYRFR5AF01ZXWF
created: 2025-10-09T15:43-03:00
sources:
  - "doc: qa-queue index review"
---

# qa-queue design review: questions index

The design review of qa-queue ended with one firm conclusion: the questions table needs an index on `(event_id, status)`. The query that builds the pending list for moderators scanned 1.2 million rows. That is far too much work for a call the moderation screen makes all the time during a live event. The rest of this note records what the review looked at, why the index has this shape, and what is still open.

## Outcome

- Add a composite index on the questions table, with `event_id` first and `status` second, written `(event_id, status)`.
- The reason is the pending-list query. It filters by event and by status, and without a matching index it scanned 1.2 million rows.
- Nothing else in the qa-queue data model was changed by the review. Other ideas came up and are listed under open questions below. None of them is decided.

If you only need the decision, that is it. The sections below are for whoever implements it or has to defend it later.

## Background

qa-queue holds the audience questions submitted during an event. Producers and community managers work through it in the moderation view. A question moves through statuses: it arrives as pending, and a moderator approves, rejects or answers it. The Phoenix side writes questions as they arrive over the WebSocket connection. It reads the pending list for the moderator UI built in Next.js. Storage is CockroachDB.

The read that matters is the pending list. It asks for every question in one event whose status is still pending, so a moderator can see what needs a decision. During a big event this read is repeated by many moderators and is also triggered when queue changes are pushed out. So it is both hot and latency-sensitive. Moderation only feels real-time if this list comes back fast.

## What the review found

The reviewers looked at the query plan for the pending-list query. The plan showed a scan over the table that read 1.2 million rows to return a small number of pending questions for a single event. The table is shared by all events, so the scan cost grows with the total history of questions, not with the size of the event being moderated. That is the wrong scaling behavior. A small event pays for the questions of every other event, and it gets worse as the product is used more.

The filter was selective on both columns, but nothing in the schema let the database use that. The existing indexes did not cover the pair, so the planner had no better route than scanning.

## Why `(event_id, status)` and in this order

- `event_id` goes first because every qa-queue read is scoped to one event. Leading with it means the database can jump straight to that event's slice of the table.
- `status` goes second because within an event the common question is which ones are pending. Equality on both columns lets the index answer the filter exactly.
- The reverse order, status first, was discussed and rejected. Pending is a low-cardinality value shared by a large share of rows across all events, so a status-first index would put the pending rows of every event together and make the event filter do the narrowing second. Event first is the better prefix.
- A single-column index on `event_id` would help, but it would still make the database read and discard rows of the wrong status for events with many handled questions. The composite index avoids that.

The index also helps any other query that filters by event alone, since `event_id` is the leading column. So we do not need a separate index for that case.

## The change

The statement is simple. It is written without an index name, because CockroachDB generates one, and the project may prefer to name it in its own migration conventions.

```sql
CREATE INDEX ON questions (event_id, status);
```

Ship it as a normal schema migration in the Elixir project, not as a hand-run statement on one cluster. After it lands, rerun the plan for the pending-list query and confirm it now uses the new index and no longer reads anything like 1.2 million rows.

## Risks and rollout

- Building an index on a large table takes time and some load. CockroachDB builds indexes online, so reads and writes keep working, but do it outside the busiest part of a live event anyway. Do not start the build while a large event is running.
- Every new question and every status change now updates one more index. The write cost is small next to the read savings, but it is real, and status changes are frequent during moderation. Watch write latency on the question insert path after the rollout.
- A status change moves a row inside the index, since `status` is part of the key. That is expected for this access pattern, but it means a burst of moderation actions touches the index more than a plain index on `event_id` would.
- Rollback is dropping the index. It does not change data, so it is safe to undo if write latency gets worse than expected.

## Open questions

- Whether the pending list should also be ordered, for example by arrival time, and whether the index should then carry that column too. The review did not settle it. Check what ordering the moderator UI really relies on before extending the index, and do not widen it without measuring.
- Whether old events should have their questions archived out of the hot table. That would also cut the scan size, but it is a larger change and was not part of this review. The index fixes the immediate problem without it.
- Whether the same shape is needed for the live poll tables. Those are a different component and a different subject, so they need their own look.

## How to verify after the change

- Run the plan for the pending-list query against a realistic data set and check that the index is used and that the row count read drops sharply from the old 1.2 million rows.
- Compare moderator view load time for a large event before and after.
- Watch the question insert path and the status update path for any slowdown from the extra index.
- If the planner still chooses a full scan on a small test database, do not conclude the index is wrong. Small tables often scan anyway. Test with data at a realistic size.

## Notes for whoever picks this up

The decision is made; the work left is the migration and the checks above. Keep the index key exactly as `(event_id, status)` unless a measurement shows a reason to change it, and write down the reason in this note if you do. If the pending-list query is rewritten later, for example to add paging, recheck that it still matches the index prefix, because a query that stops filtering on both columns will quietly go back to scanning.
