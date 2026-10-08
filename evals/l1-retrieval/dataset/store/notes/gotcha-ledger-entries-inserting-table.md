---
id: 01JTXTSAMKPZ13PVKWMF25XDVP
created: 2025-05-10T16:29-03:00
---

# ledger-entries-table insert fails when the monthly partition is missing

Inserting into ledger-entries-table fails with the PostgreSQL error no partition of relation found for row when the monthly partition has not been created yet. The error text contains `no partition of relation`, and that string is what to search logs for. The table is partitioned by time, one partition per month. If a row's date falls in a month that has no partition, PostgreSQL has nowhere to put the row and rejects the write. It does not fall back to anything. This note covers how it shows up, why it happens, what to check first and how to avoid it. Read it before you spend an hour blaming the Go code.

## Symptom

The write to ledger-entries-table fails and the error text contains `no partition of relation`. The full message continues with the table name and says that no partition was found for the row. The failure is immediate and deterministic. Retrying the same row without changing anything gives the same error every time.

It usually appears at a month boundary, or when someone replays or backfills data for a month that was never set up. Rows for months that already have partitions keep going in normally. So the same job can look half healthy: some entries land and others are rejected, depending on the date each one carries.

In the ingestion service the error comes back through the Go database driver as an ordinary error from the statement. It is not a connection problem, a timeout or a lock wait. The service logs it with the failing entry and then, depending on the path, either returns a gRPC error to the caller or stops consuming and leaves the Kafka offset where it was.

## Why it happens

ledger-entries-table is a partitioned parent table. The parent holds no rows itself. Each monthly partition is a separate child table that covers one month of the partition key. On insert, PostgreSQL routes the row to the child whose range contains the key value. When no child covers that value, you get the error above.

Nothing creates partitions on its own. PostgreSQL has no built-in automatic partition creation for range partitioning. Someone or something has to create next month's partition before the first row for that month arrives. If that step is late, skipped or broken, the first insert for the new month fails.

A default partition would catch stray rows, but the choice not to rely on one was deliberate. A catch-all partition hides the problem and makes later cleanup harder, because rows end up in the wrong place and splitting the default partition later is painful. Check the current table definition before assuming either way.

## Which date decides the partition

The row's partition key value decides where it goes, not the time the insert runs. This matters because a settlement file can arrive late and carry entries dated in an earlier month. Those rows need the earlier month's partition, which is an easy case to forget. The same applies to entries dated slightly ahead of the current month, for example when a processor reports a settlement date for the next period.

So the failure can happen mid-month. It does not need a month rollover. A late file for a past month, or a future-dated entry, is enough. When you see the error, look at the date on the failing row first. Do not assume the current month is the missing one.

Time zones can shift a row across a boundary. An entry stamped near midnight at month end may land in the next month once the value is converted to the time zone the table uses. Check which zone the key is stored in before deciding that a row is dated wrongly.

## First checks when you see it

Start with the failing row's date, then check whether a partition exists for that month. Look at the list of child tables attached to the parent in PostgreSQL. Compare the covered ranges with the date of the row. If there is a gap, that is your answer.

Then ask three questions. Did the job that creates partitions run? Did it fail quietly? Is the row's date reasonable, or is it a bad value from upstream such as a wrong year? A date far in the past or future usually means a parsing bug or bad data in the settlement file, and creating a partition for it would be the wrong fix.

Only after that decide whether to create the partition or reject the row. Creating partitions for nonsense dates just clutters the schema and hides the data problem.

## Fixing it right now

Create the missing monthly partition for ledger-entries-table, attached to the parent, with bounds that match the neighbouring partitions exactly. Match the naming convention used by the existing children so the tooling and the people reading the schema are not surprised. Ranges must not overlap and should not leave a gap. After the partition exists, the next insert for that month works with no restart.

If you are in a hurry, copy the shape of the previous month's partition. Check ownership and permissions on the new child. A partition created by a different role than usual can leave the application role unable to write to it, and you then get a permissions error that looks like a different bug.

After creating it, replay what failed. Do not assume the rejected rows were saved somewhere. See the next section.

## What happens to the failed rows

A rejected insert writes nothing. The row is not stored in a side table and not queued by PostgreSQL. Whether it can be recovered depends on the caller.

If the entry came from a Kafka consumer that did not commit its offset, the message is still on the topic and a restart or retry will pick it up once the partition exists. If the consumer committed the offset before the write, or skipped the message after an error, the entry is gone from the consumer's point of view and has to be replayed from the topic or from the original settlement file.

If the entry came in through a gRPC call, the caller got an error and owns the retry. Check whether the caller retries at all. Some do not, and the entry then exists only in the caller's own records.

Before declaring the incident closed, compare counts between the source and ledger-entries-table for the affected period. A reconciliation tool that silently misses entries is worse than one that fails loudly.

## Effect on reconciliation

Ledgerlark matches processor settlement files against internal ledger entries. If ledger entries are missing because of this error, the matching step sees settlement lines with no counterpart. Those show up as mismatches and are flagged for review. Reviewers on the finance operations side will see a burst of apparent unmatched settlements that are not real discrepancies.

Tell them early. Otherwise they will spend time chasing individual items that resolve themselves after the partition is created and the entries are replayed. Once the data is back, rerun matching for the affected period so the false flags clear, and check that nobody closed or annotated those items in the meantime based on the bad picture.

## Prevention: create partitions ahead of time

The durable fix is to create partitions before they are needed, with a comfortable margin, not on the day. Whatever job does this should create several months ahead, not just the next one, so that a missed run does not cause an outage immediately.

Make the job idempotent. Running it twice must not fail or create duplicates. Make it report clearly when it fails. A silent failure in this job is how the problem returns, and nobody notices until an insert is rejected. Alert on the job's failure, and also alert on the gap itself: if the furthest covered month is closer than the agreed margin, raise it.

Keep the partition definitions in the same place as other schema changes so that they are reviewed and reproducible, and so a fresh environment gets them too.

## Terraform and environments

The infrastructure is managed with Terraform, but partitions are data-layer objects inside PostgreSQL, and it is easy to assume Terraform covers them. Check what your Terraform actually manages. If it provisions the database and roles but not the child tables, partitions have to come from the schema migration path or the scheduled job instead.

This matters for new environments. A freshly provisioned database that has the parent table but only a few partitions will work for a while and then fail with `no partition of relation` once the covered months run out. Staging and test databases are the usual victims, because nobody watches them. A test suite that passes for weeks and then fails on the first day of a month is the classic sign.

If a test fails on a date boundary, suspect this before suspecting the test.

## Local development and tests

Tests that insert into ledger-entries-table with the current date depend on a partition for the current month. Make test setup create the partitions it needs, for the dates it uses, instead of relying on whatever the shared database holds. Fixtures with hard-coded dates are fine only if their months are created in setup too.

For local databases, seed the partitions as part of the usual bootstrap. A developer who restores an old dump and then runs recent data will hit this error. The message does not mention months, so people lose time on it.

If you write Go tests that cover month-boundary behaviour, create both sides of the boundary explicitly, and add one case where the second side is missing, to confirm the error is surfaced properly and not swallowed.

## How the Go service should handle the error

Treat this error as non-transient but fixable. Blind retry loops will just spin, fill the logs and delay the consumer. Better behaviour is to log it with enough context to act on: the date of the row, the table, and the fact that the partition is missing. Then stop or pause the affected work and raise an alert, instead of dropping the entry.

Do not auto-create partitions from the request path as a reflex. It looks convenient, but it needs elevated privileges for the application role, can race between concurrent writers, and can take locks on the parent while traffic is running. A scheduled job with a proper margin is safer. If you do consider on-demand creation, handle the case where two workers try at once.

If a bad date is the cause, send the entry to review or a dead-letter path instead of creating schema for it.

## Kafka consumer behaviour

In the Kafka consumers, the important choice is when the offset is committed. Commit after the database write succeeds. Then a failure from this error leaves the message uncommitted, and it is processed again once the partition is in place.

Be careful with consumers that skip poison messages after some number of failures. A missing partition looks like a poison message to such logic, but it affects every message for that month, not one. Skipping them loses a whole month of entries. Prefer pausing the partition of the topic, or the whole consumer, and alerting a human.

Lag will grow while the consumer is stuck. That is expected and is a useful alarm. Do not clear it by resetting offsets forward.

## Things that look similar but are not

Several other failures get confused with this one. A unique violation on an entry is a duplicate problem, not a partition problem. A permission denied error on a new child table is an ownership problem. A constraint failure on a row that does have a partition is a data problem. A connection or timeout error says nothing about partitions.

The one specific signal is the text `no partition of relation`. If the message has that text, the answer is always that no child table covers the key value of the row. The only open question is whether the partition should exist or the date is wrong.

Also keep this apart from migrations that fail partway. A migration that changed the parent but did not finish attaching children can produce missing coverage for a similar reason, though the cause is different.

## Detaching and dropping old partitions

The opposite mistake exists too. If an old partition is detached or dropped as part of retention cleanup, later inserts for that old month fail with the same error. This shows up when a late or corrected settlement file is processed for a month already archived.

Before removing a partition, confirm that nothing still writes to that period, and agree with finance operations on how late corrections can arrive. Archive the data before dropping, not after. If a correction for an archived month is genuinely needed, recreate the partition, load the entry, and then decide whether to keep the partition or detach it again.

## Checklist

When the error appears: confirm the text contains `no partition of relation`; read the date on the failing row; check whether that date is sane; list the existing partitions and find the gap; create the missing partition with matching bounds and the right ownership; replay the rejected entries; compare counts for the period; rerun matching so false mismatches clear; tell the reviewers.

Afterwards: find out why the partition was missing, fix the creation job or its alerting, and add margin. If it was a bad date from upstream, fix the parser or reject at the boundary.

## Open questions

Some things are not settled in this note and should be checked against the current schema and jobs: whether the table has a default partition at present, which job or migration path owns partition creation in each environment, how many months ahead it creates, and who gets alerted when it fails. Fill these in here when you find out, so the next person does not have to look again.
