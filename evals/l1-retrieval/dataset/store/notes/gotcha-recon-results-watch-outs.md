---
id: 01K4RQ58NF0HZBWKGVA3AQ0QVM
created: 2025-09-09T23:27-03:00
---

# Gotchas when changing recon-results-table

Notes for anyone touching recon-results-table, the place where Ledgerlark keeps the outcome of comparing card-processor settlement files with internal ledger entries. Everything here is general. Check the actual schema, the actual consumers and the actual deployment before you trust any of it. I wrote it quickly after getting burned by assumptions, so it is blunt.

The short version: this table looks like a plain result store, but it sits in the middle of a lot of things. Finance operations people read what it says and act on it. Review queues are built from it. Other services write to it from different directions. A change that is harmless in a unit test can quietly change what a reviewer sees, or hide a mismatch that should have been flagged. Treat it as shared, long-lived, and audit-relevant.

```text
settlement file -> Go ingest -> Kafka -> Go matcher -> recon-results-table (PostgreSQL) -> gRPC -> review
```

That diagram is a simplification. The real paths have retries, replays and manual corrections that all end up in the same place.

## What the table actually represents

Before changing anything, be clear about what a row means. A row is not just a record that two things were compared. It is the system's current claim about the relationship between a settlement item and a ledger entry (or the lack of one). That claim can be matched, partially matched, mismatched, missing on one side, duplicated, or awaiting more data. Each of those has a different meaning to the person on the review queue.

Things that follow from this:

- A row can be superseded. A later settlement file, a late ledger posting or a reviewer decision can change the claim. If you add a column or a state, think about what happens when the claim is revised, not just when it is first written.
- Absence of a row is also information. Some consumers treat "nothing written yet" as "not yet processed", others as "nothing to flag". If your change alters when rows get written (earlier, later, batched differently), you change the meaning of absence for them.
- The status values are a vocabulary shared with people. Renaming or splitting a status is a product change, not a refactor. Finance ops will have dashboards, saved filters and written procedures that use the words.
- Do not assume that one settlement item maps to one row. Splits, partial captures, refunds, chargebacks, fees and adjustments all make the relationship messy. Check how the current code deals with those before you simplify anything.

If you cannot say in a sentence what a row means after your change, stop and work that out first.

## Schema changes in PostgreSQL

The table is likely big and busy. Schema changes that look cheap on a dev database can be slow or blocking in production.

- Adding a column with a default, adding a constraint, changing a type and building an index can each take locks or rewrite data depending on how they are written. Read what the specific operation does on the PostgreSQL version actually deployed, not what you remember.
- Build indexes in a way that does not block writers. Plan for the build to be slow and to possibly fail partway, leaving something invalid that needs to be cleaned up.
- Adding a NOT NULL constraint or a foreign key means validating existing rows. Old rows were written by older code and may break the rule. Look at real data shape, including odd historical rows, before you assume it is clean.
- Dropping a column is a one-way door for any consumer that still reads it. Remove readers first, wait, then drop. Do not do it in the same release.
- Changing a column's type tends to ripple through the Go structs, the gRPC messages, the Kafka payloads and any reporting queries. Search all of them, not just the service you are in.
- Money amounts: never switch representation casually. Rounding, scale and currency handling are exactly where reconciliation mismatches come from. A representation change can create or erase mismatches by itself.
- Time columns: be explicit about whether a value is a settlement time, a ledger posting time, a processing time or a review time. Time zones and cut-off conventions differ between processors. Mixing them up produces mismatches that look real.

Prefer the expand, migrate, contract pattern. Add the new shape, write both, backfill, switch readers, stop writing the old shape, then remove it. Each step should be deployable and revertible alone.

## Migrations and Terraform

Schema is only half of it. The database itself, its parameters, its users, its replicas and its networking are probably managed with Terraform, and that is a separate change path from application migrations.

- Do not mix an infrastructure change and a data-shape change in one rollout unless you have to. When something goes wrong you want to know which one did it.
- Check how migrations are run: by the service at startup, by a separate job, or by hand. Startup migrations in a multi-instance Go service can race. Make sure only a single runner applies them, or that the tool locks.
- A migration that runs long can exceed timeouts set elsewhere (connection pools, load balancer, job runner, Terraform provisioners). Find those limits before you start.
- Roles and grants: new tables, views or columns may need grants for read-only roles used by reporting and review tooling. Forgetting this shows up as a quiet failure in someone else's dashboard, days later.
- If Terraform manages parameters like statement timeouts or connection limits, changing them affects every user of the database, not only this table. Read the plan output carefully and do not accept an apply that touches things you did not intend.
- Keep environments aligned. If staging was hand-edited at some point, a migration that works there may not behave in production, and the other way round.

## Writers: idempotency and duplicates

The path into recon-results-table is driven by Kafka and Go consumers. Kafka delivery is at least once in practice, so any writer must tolerate seeing the same input again. This is the most common source of trouble.

- Writes need a natural key or a deduplication rule that is stable across retries and replays. If you change what the key is made of, you can create duplicates for existing data or collapse rows that should be separate.
- Upserts need care. Decide which fields a repeat write may overwrite and which it must leave alone. A replayed old event must not overwrite a newer outcome or a reviewer's decision.
- Ordering is not guaranteed across partitions, and may not be guaranteed even within a partition when consumers are rebalanced or when there are several producers. Do not assume the last event processed is the latest event in business terms. Compare an explicit version or timestamp where one exists.
- Consumer rebalances and restarts happen during deploys. A half-processed batch gets redelivered. If your change makes a write non-repeatable (a counter increment, an append-only side effect, a notification sent per write), redelivery will duplicate it.
- Transactions: be clear about what commits together. Committing the Kafka offset before the database write loses data on a crash. Committing after means a repeat on a crash. The second is usually right, which is another reason for idempotent writes.
- Batch size and transaction size affect lock time and replication lag. A change that makes writers hold locks longer can stall other writers and the review readers.
- Poison messages: decide what a writer does with an input it cannot process. Blocking the partition forever and silently skipping are both bad. Whatever exists today, do not weaken it without noticing.

## Readers: review queues, gRPC and reports

The table is read by the review flow, through gRPC services, by ad hoc queries from finance operations and by scheduled reports. You usually know about the first and miss the rest.

- Search for every query touching the table, including views, materialized views, saved reports and anything in other repositories. If you cannot search them all, ask the people who own them before you change shapes.
- Pagination and ordering of the review list must stay stable. If you add a state or change a sort key, a reviewer can see rows jump or disappear between pages, or see a row twice. Prefer keyset pagination over offsets for a table that keeps changing.
- gRPC messages are a contract. Adding fields is generally safe if clients ignore unknown ones; renaming, renumbering, reusing a field tag or changing a type is not. Keep old clients working through a full deploy cycle and longer, because some clients are deployed on their own schedules.
- Enum-like values in gRPC need an explicit unknown or default handling. A client that receives a status it does not know should not crash, and should not treat it as "matched".
- Default values: a missing field in a message reads as the zero value in Go. Zero can mean "false", "empty" or "not reviewed", and each of those can be misread. Check what the zero value means for every field you add.
- Reads against replicas may lag. If a reviewer takes an action and the page reloads from a lagging replica, the old status shows up and they act again. Be careful about where read-after-write behavior matters.
- Heavy reports can hurt writers. If you add a new access pattern, check the query plan on realistic data volume, not on a development copy.

## Matching logic and what counts as a mismatch

The table stores the result of matching, so any change in the matching rules changes what ends up in it. Even a small tolerance tweak changes what gets flagged for human review, which changes the workload of an entire team.

- Tolerances for amounts, dates and descriptors are business rules. Do not alter them as a side effect of cleanup, a library upgrade or a type change.
- Changes in rounding, currency conversion or fee handling can make previously matched rows look mismatched, or the reverse. The reverse is worse, because nobody reviews a row that has been marked clean.
- When a rule changes, decide what happens to existing rows. Leave them as they were, recompute them, or mark them with the rule that produced them. Leaving them silently inconsistent makes later analysis confusing.
- If you recompute historic rows, remember that reviewers may already have resolved some of them. Recomputation must not undo or hide reviewer decisions or the trail of how a row got its status.
- Settlement files from different processors differ in format, in timing and in what they call things. A change tested against one processor's data can break another's. Test with several, including awkward samples: empty files, files with only adjustments, files with repeated items, files that arrive late or twice.
- Late and corrected files happen. Make sure a later correction produces a clear update to the right rows rather than a parallel set of rows.
- Keep a row able to explain itself. If your change removes the information about why something was matched or flagged, support and reviewers lose the ability to check the system. Prefer adding explanation fields over removing them.

## Backfills, replays and large data operations

Sooner or later someone needs to rewrite many rows: a bug fix, a new column, a rule change. These operations are risky in this table.

- Run them in small batches with a pause between, and make them resumable. A giant single statement can lock, bloat and lag replicas, and leaves you with nothing if it fails late.
- Make them idempotent, so you can stop and run again. Record progress somewhere that survives a crash.
- Mind autovacuum and bloat. Updating a large share of rows creates dead tuples, and the table and its indexes may grow before they shrink. Watch disk and replication while it runs.
- Replaying Kafka topics into the table repeats history through the same writers. Check that the writers handle old events correctly, that they do not trigger downstream notifications again, and that they do not overwrite newer state.
- Mark data created by a backfill or replay so it can be told apart later. If a backfill itself turns out to be wrong, you want to be able to find its output.
- Do a dry run that only reports what would change and read the report as if you were a finance person. If the proposed changes would surprise them, stop.
- Never run an ad hoc data fix directly in production without a second pair of eyes and a way back. Take a copy of the affected rows first.

## Retention, privacy and audit

The data here is financial and derived from card processor files. It is also something auditors and finance teams may need to trust and explain long after the fact.

- Do not delete or compact rows on your own initiative. Retention rules come from finance, legal and contracts, and the table may serve as evidence of what was reconciled and when.
- Check what sensitive data may be copied into the table: card references, merchant details, customer identifiers. Adding a column that copies more of the source record increases what has to be protected, masked in logs and limited in access.
- Do not log full rows in error handling. It is tempting during debugging and it ends up in a log store with different access rules.
- Keep history of status changes where it exists. An update that overwrites a status without leaving a trace may break the audit trail. If you change how updates work, confirm that the trail still records who or what changed a row and why.
- Partitioning or archiving schemes help size and speed but change how queries, constraints and uniqueness work. Unique constraints in a partitioned table have restrictions, so a plan that relies on them needs to be checked early, not at the end.

## Rollout, testing and watching it afterwards

A change here is not finished when tests pass. The effects show up in the data, over days, in the review queue.

- Test with realistic data shape and volume. Synthetic rows with tidy values will hide the cases that matter: nulls, odd currencies, very old rows, very long text, duplicates.
- Add tests for repeat delivery and out-of-order delivery of the same input, not just the happy path.
- Roll out in steps. Writers first when the new shape is additive, readers after. For breaking changes, put the compatibility layer in before the break.
- Use a flag or a configuration switch for behavior changes in matching, so you can turn them off without a redeploy of everything.
- Compare before and after. For some period, run the old and new logic side by side and diff the outcomes. Unexpected differences are the point of the exercise.
- Watch the numbers that tell you something is off: the share of rows flagged for review, the volume waiting in the queue, the lag of the consumers, write errors, query latency, replication lag and table size. A sudden drop in flagged rows can be as bad as a spike.
- Tell the finance operations team before a change that alters what they see, and tell them after, with what to expect. Surprise in the review queue costs them real time.
- Have a rollback that you have actually thought through. Application rollback is easy; data already written in a new shape or with new rules is not. Know what the older code does when it meets rows written by the newer code.

## Things I keep forgetting

- Other repositories and jobs may touch the table without being in the service you are editing.
- The tests in one service do not cover the contract with consumers elsewhere.
- The dev database is smaller, cleaner and less concurrent than production in every way that matters.
- A column that looks unused may only be used by a monthly report.
- A status that looks dead may be used by an old procedure that people still follow.
- Timeouts live in many places, and the shortest one wins.
- Fixing the symptom in the table, rather than the writer or the matching rule that produced it, brings the problem back on the next file.
- When in doubt about the meaning of a field or a status, ask the people who use it before you decide.

Add to this note whenever a change in recon-results-table turns up a trap that is not listed here.
