---
id: 01M2JXXAG2X4QKF9VWQP5R09PE
created: 2026-09-15T13:20-03:00
---

# results-ledger stores votes as append-only rows

We decided that results-ledger records every vote as its own `append-only` row, and a rollup job sums those rows into the totals people see. We do not keep a counter per option and update it on each vote. The reason is hot-range contention on a single key in CockroachDB. This note records the decision, the reasoning, what we gave up, and what to check before changing it.

## Decision

In results-ledger, a vote is an insert and nothing else. Rows are never updated and never deleted by the vote path. Totals for a poll option come from a rollup job that sums the rows. The live results screen reads the rolled-up values, not the raw rows.

The rule for anyone touching the code: do not add an update-in-place counter to results-ledger, even as a shortcut or a cache that the vote path writes to. If a number needs to be fast to read, the rollup job produces it. The vote path does not.

## Why not a counter

The obvious design is one row per poll option with a count column, incremented per vote. In a large virtual event, a single popular option can get a burst of votes within a very short time. Every one of those increments hits the same key.

CockroachDB keeps data in ranges, and a single key lives in a single range with one leaseholder. Writes to one key serialize through that leaseholder. Concurrent transactions touching the same row also conflict, so they wait on each other or restart. Under a burst, latency climbs, transactions retry, and the vote path backs up. Adding nodes does not help, because the hot key still lives in one place.

This is the hot-range contention problem. A counter makes the busiest moment of the event the slowest moment of the database.

## Why append-only rows fix it

With `append-only` rows, each vote is a new key. Inserts for different votes do not touch the same row, so they do not conflict with each other. If the key design spreads inserts across the keyspace, the load also spreads across ranges and leaseholders, and adding nodes helps.

The cost of contention moves from the write path to the read side, where we can control it. The rollup job reads and sums on its own schedule, and it can fall behind without blocking anyone from voting.

## Key design for the rows

The point of the design is lost if the primary key makes all new rows land at one end of the keyspace. A key that begins with a monotonically increasing value, such as a timestamp or a sequence, sends every insert to the last range, and we would have rebuilt the hot spot in a different place.

The key should therefore lead with something that spreads writes, not with time. Keep this in mind when reading or changing the schema. The exact column layout lives in the migrations, and this note does not repeat it. If you change the key, check where new inserts land under a burst, not just whether queries still work.

## The rollup job

The rollup job sums the vote rows per poll option and writes the result to where the display reads from. It is the only writer of the summed values. That gives each summed value a single writer, which also keeps those rows from becoming a contention point.

The job should be able to resume from where it stopped and should not double count. Because the vote rows never change, a job can remember how far it has read and add only what is new. If it is ever unsure, it can recompute a poll from scratch and get the same answer, because the source rows are the truth.

## Read path and freshness

The displayed totals lag the votes by roughly one rollup cycle. For a live poll on a big stream, this is acceptable. Viewers see numbers move in steps, not on every single vote.

We chose this tradeoff on purpose. If a producer asks why the bar did not move the moment they voted, the answer is that results are rolled up, not read live from raw rows. If the lag ever becomes a product problem, shorten the rollup interval first. Do not move to a counter.

## Delivery to clients

Phoenix channels push updated totals to participants over WebSockets, and the Next.js front end renders them. The push is driven by rollup output, not by individual vote inserts. That means the number of messages to clients scales with rollup cycles and not with vote volume, which is a side benefit: a burst of votes does not become a burst of broadcasts.

Anything that wants to show a result to a user should go through this same path so every client sees the same totals.

## Moderation interaction

Real-time moderation exists for Q&A and polls. If a moderator removes a poll, or invalidates votes (for example after spam or brigading), the `append-only` rule means we do not delete rows from the vote path. Instead the invalidation is recorded as its own fact, and the rollup honors it, either by excluding the affected rows or by writing compensating rows.

This keeps the history intact for review and keeps the write path free of updates. When designing a new moderation action that affects results, follow the same pattern: add a record, let the rollup interpret it.

## Correctness and audit

Because rows are never changed, the ledger doubles as an audit trail. Event producers who dispute a result can be shown the votes that made it up. Any total can be rebuilt from the rows, so a bug in the rollup is fixable by fixing the job and recomputing, with no lost data.

This was a reason for the name results-ledger in the first place: it is a log of what happened, and totals are derived from it.

## Duplicate votes

Appending rows makes it easy to accept the same vote twice if a client retries after a timeout. The insert path needs an idempotency guard so a retried request does not become a second vote. That guard should be a uniqueness rule on the row, not a read-then-write check, since read-then-write under concurrency is itself a source of conflicts and races.

Eligibility rules, such as one vote per participant per poll, are enforced at the same place. Check how the current insert path does this before changing it.

## Storage growth

One row per vote means the table grows with total votes, not with the number of options. For large events that is a lot of rows, but each is small, and the cost is predictable. We accepted this in return for no contention.

If storage ever matters, the answer is to archive or compact old, finished polls after their final totals are settled. It is not to start updating live rows. Compaction of closed polls is safe to do later because nothing is writing to them anymore.

## What we gave up

- Reads are slightly stale compared to a counter.
- There is a background job to run, watch, and keep correct.
- Storage grows with every vote.
- A reader of the raw table cannot see a total directly; they must sum or read the rolled-up output.
- The insert path needs its own duplicate protection.

We judged these cheaper than slow or failing votes during the busiest moments of an event.

## Alternatives considered

A single counter row per option was rejected for the contention reasons above.

Sharded counters, with several counter rows per option that are picked at random and summed on read, reduce the problem but keep update-in-place semantics, retries on conflict, and the question of how many shards are enough. They also lose the per-vote history. We preferred the plain `append-only` approach, which is simpler to reason about.

An in-memory counter in the Elixir processes, flushed to the database periodically, was also considered. It is fast, but a crash loses votes unless every vote is also stored, at which point we are back to storing rows anyway. It also complicates running more than one node.

## What to watch in production

- Rollup lag: how far the displayed totals are behind the inserted rows. A growing lag means the job is under-provisioned or stuck.
- Retries and latency on the vote insert. They should stay flat during a burst. If they rise, check the key design first, then check whether something is writing to a shared row.
- Range hotspots in the CockroachDB console during large events.
- Mismatch between a full recompute and the rolled-up total for a sample of polls.

## Rules for future changes

- Do not add a shared counter or any row that every vote updates.
- Keep the rollup job as the only writer of summed values.
- Keep the insert path idempotent.
- Express moderation effects on results as new records.
- If you change the primary key, test under a burst and look at where the inserts land.
- If you want fresher numbers, shorten the rollup interval before anything else.

## Open questions

- When to compact or archive finished polls, and who triggers it.
- Whether very large events need the rollup split by poll so one big poll cannot delay the others.
- Whether the front end should show that totals are updating in steps, so producers are not surprised.
