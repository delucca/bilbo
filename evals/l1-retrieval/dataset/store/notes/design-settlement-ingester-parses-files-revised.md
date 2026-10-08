---
id: 01M0FC6RBTWYMG0Z5J7PHZXX1N
created: 2026-08-20T07:41-03:00
---

# settlement-ingester design: batch publishing of parsed rows

This note replaces the earlier note "settlement ingester parses files". The new value is that settlement-ingester publishes parsed rows in batches of 1000 rows, and that replaces the earlier batch size. Everything below describes the component as it stands with that change. Where I am not sure of a detail, I say so instead of guessing.

## What settlement-ingester does

settlement-ingester is the entry point of Ledgerlark. Card processors drop settlement files, and the ingester reads each one, parses it into rows, and hands those rows to the rest of the system. It does not decide whether a row matches a ledger entry. That is the job of the reconciliation side, which compares the parsed rows against internal ledger entries held in PostgreSQL and flags mismatches for the finance operations team to review.

The ingester is written in Go. It publishes to Apache Kafka. Other services reach it over gRPC for control and status calls, and its infrastructure is declared in Terraform. Finance operations teams at online marketplaces are the end users, but they never talk to the ingester directly. They see its effects as rows that show up for reconciliation, or as files that are late or stuck.

The short version of the change: the ingester now groups parsed rows into batches of 1000 rows before publishing each batch to Kafka. A batch is the unit of publishing. It is not the unit of parsing, and it is not the unit of reconciliation.

## The batch size change

The rule is simple. The ingester accumulates parsed rows from a settlement file, and when it holds 1000 rows it publishes them as one batch. The last batch of a file is usually smaller, because a file rarely holds an exact multiple of the batch size. That short final batch is published as is, and the ingester must not wait for more rows, since no more will come from that file.

The earlier batch size is gone. I am not recording the old figure here on purpose: it no longer applies, and anyone reading an old log line or an old dashboard should treat it as history. If you find code, config or documentation that still assumes the old size, it is stale and should be fixed to match 1000 rows.

Why the change was made, as far as I know:

- Fewer, larger publishes mean less per-message overhead on the Kafka side. The earlier size produced more messages than the brokers needed to handle for the same data.
- Consumers do their database work per batch, and a larger batch lets them use fewer round trips to PostgreSQL for the same number of rows.
- A batch of 1000 rows is still small enough that one failed batch can be retried without redoing a large part of a file.

I did not measure these effects myself in this session. They are the reasoning behind the choice, not benchmark results. If someone wants to move the size again, they should measure first and update this note with what they found.

## Flow from file to Kafka

A rough picture of the path a settlement file takes:

```text
settlement file -> settlement-ingester -> Kafka (batches of 1000 rows) -> reconciliation -> PostgreSQL
```

In words:

- A settlement file from a card processor arrives in storage that the ingester watches or is told about.
- settlement-ingester opens the file and parses it row by row. Parsing is streaming, so the whole file is never held in memory at once. This matters for large files, and it is the reason batching is done in the ingester and not later.
- Each parsed row is checked for basic shape: required fields present, amounts readable, dates readable. A row that fails this check is not silently dropped. It is recorded as a parse failure with enough context to find it again in the source file.
- Good rows are added to the current batch. When the batch holds 1000 rows, it is published to Kafka and a new batch starts.
- At the end of the file the remaining rows are published as the final, shorter batch, and the file is marked as fully read.
- Reconciliation consumers read the batches from Kafka, compare rows against ledger entries in PostgreSQL, and write mismatches for review.

Ordering inside a file matters less than it might seem, because reconciliation matches on content and not on position. Still, the ingester keeps rows of one file in the order it read them within a batch, and it publishes the batches of one file in order. Do not rely on strict order across different files.

## Failure handling and retries

Batching changes what a failure costs, so this part is worth reading before touching the publish code.

If a publish to Kafka fails, the ingester retries that batch. The batch is the retry unit. It does not re-parse the file from the start, and it does not drop the batch. Because a batch holds up to 1000 rows, a retry resends at most that many rows. The consumer side has to tolerate seeing the same batch twice, since a publish that looked like a failure to the ingester may in fact have reached the broker. Duplicate protection lives in the reconciliation consumers, which key rows by content from the settlement file and not by batch. Do not move that protection into the ingester without agreeing it with the reconciliation owners.

If the process dies partway through a file, the file is not considered done. On restart the ingester starts the file again. Rows already published will be published again, and the same duplicate tolerance covers that. This is slightly wasteful but safe, and I prefer it to a checkpoint scheme that could lose rows. If checkpointing is ever added, it needs to record the last fully published batch and nothing finer.

Parse failures are separate from publish failures. A file with some bad rows still publishes its good rows. The bad rows are reported so that an operator can see them, and the file ends up flagged as partly failed instead of clean. A file that cannot be opened or recognized at all publishes nothing and is flagged as failed as a whole.

On the gRPC side, status calls report progress per file in terms of rows parsed and batches published. After the batch size change, anyone reading those counts should remember that one published batch stands for up to 1000 rows, not one row and not the old size.

## Interactions with other parts

Kafka: batches are the message payload. A larger batch means larger messages than before, so the broker and topic settings for maximum message size must allow a batch of 1000 rows with the widest rows we expect. Those limits are part of the Terraform that manages the topics and the brokers' client-facing settings. If a batch is rejected as too large, the fix is in the Terraform-managed limit or in the row width assumptions, and not in silently shrinking the batch size. Shrinking it quietly would make this note wrong.

PostgreSQL: the ingester itself does not write ledger data. It may keep small bookkeeping about files and their state. The heavy writes, matches and mismatches, are done by reconciliation. A bigger batch means each consumer transaction can cover more rows, so transaction size on the PostgreSQL side went up with the change. Nobody has reported a problem, but it is the first thing to look at if reconciliation latency or lock waits grow after a deploy.

gRPC: the control surface did not change shape with this work. Callers still ask the ingester to start, inspect or cancel work on a file. Only the meaning of batch counts in status responses shifted, as described above.

Terraform: the batch size is a runtime setting of the service, and the infrastructure code around it should not hard-code a different figure. If a Terraform variable or module description mentions a batch size, it should say 1000 rows or point at the service setting instead of repeating a number that can drift.

## Tuning, risks and open questions

Risks I can name:

- Memory: the ingester holds one batch in memory while filling it. At 1000 rows this is modest, but wide rows or many files processed in parallel multiply it. Watch memory if the number of concurrent files grows.
- Latency to first result: a bigger batch delays when the first rows of a file reach reconciliation, since the ingester waits to fill a batch. For normal files this is negligible. For tiny files the short final batch goes out at end of file, so nothing waits long.
- Message size limits on Kafka, covered above. This is the most likely cause of a surprise after a deploy.
- Stale assumptions: dashboards, alerts and runbooks written for the earlier batch size may compute rates or thresholds from it. They need a pass.

Open questions:

- Should the batch size be a fixed value or settable per environment? Right now the intent is one value, 1000 rows, everywhere. A per-environment override would be easy to add but makes behavior harder to compare between environments.
- Should a batch also be flushed on a time limit, so a slow file does not hold rows back? Not done. Files are read from storage, not streamed live, so rows do not trickle in and the case has not come up.
- Whether to add checkpointing for very large files. Not needed so far, because restarting a file is safe and the duplicate protection downstream handles it.

## What to do when you change this again

If you change the batch size, or anything in how batches are formed or published:

- Update this note in place instead of writing a new one on the same component, and say what the new value replaces.
- Check the Kafka message size limits in Terraform against the new size and the widest rows.
- Tell the reconciliation owners, because their transaction sizes follow the batch size.
- Re-read the status counts exposed over gRPC and make sure their meaning is still stated correctly in any runbook.
- Search for the old figure in code, config, dashboards and docs, and fix every place that still assumes it.

The current, agreed state is: settlement-ingester publishes parsed rows in batches of 1000 rows.
