---
id: 01KKZABWNY4C3G68Z5T7F69SAW
created: 2026-03-17T22:53-03:00
---

# settlement-events dead-letter topic plan

The decision: create the dead-letter topic `lark.settlement.dlq` for settlement-events before `2026-12-01`, so that unparseable rows stop blocking consumers. Today a single bad row in a settlement file can hold up the consumer group reading settlement-events. After the topic exists, a row that cannot be parsed is sent there and the consumer moves on. The deadline is `2026-12-01`. Anything that depends on clean, unblocked settlement-events consumption should assume this topic is in place by then.

This note is a plan. It lists the steps, who touches what, and what to check. It does not fix every detail; the general shape is enough to start work.

## Why this is needed

Ledgerlark reconciles card-processor settlement files against internal ledger entries and flags mismatches for review. Finance operations teams at online marketplaces rely on that flow. The settlement-events stream carries the parsed rows of those files through Apache Kafka to the services that match them against the ledger in PostgreSQL.

Processor files are not always well formed. Some rows have a missing field, a broken amount, a date in an unexpected format, or an encoding problem. The parser rejects those rows. At the moment a rejected row has nowhere to go, so the consumer either retries it forever or stops. Either way the rows behind it wait, and reconciliation for that processor falls behind. Finance teams then see late or missing mismatch flags, which is the thing the product is meant to prevent.

The fix is a dead-letter topic: `lark.settlement.dlq`. Unparseable rows are published there with enough context to look at them later, and the main flow keeps going.

## Scope

In scope for settlement-events:

- Creating `lark.settlement.dlq` in every environment that runs the settlement-events consumers.
- Changing the Go consumer so that a row it cannot parse is published to `lark.settlement.dlq` and then acknowledged, instead of blocking.
- Recording why the row failed, and where it came from, in the dead-letter message.
- A way for an operator to see what is sitting in the dead-letter topic.

Out of scope for now:

- Automatic replay of dead-lettered rows. That can come later once we know what the common failures look like.
- Changes to how the processors produce files.
- Any change to the ledger side.

## Steps

1. Define the topic in Terraform alongside the other Kafka topics, with the name `lark.settlement.dlq`. Keep retention long enough that a finance team can look at failures after a weekend or a holiday. Pick partitions and replication to match the main settlement-events topic; do not invent a new scheme.
2. Apply the Terraform change in the lowest environment first, check that the topic exists, then promote it through the others. Do not leave the production step for the last week before `2026-12-01`.
3. Update the Go consumer. On a parse failure it should build a dead-letter message, publish it to `lark.settlement.dlq`, and only then commit the offset of the bad row. If the publish to the dead-letter topic itself fails, do not commit; treat that as a real error, because otherwise a row is lost silently.
4. Add the failure context to the message. Keep the original bytes of the row untouched, and put the reason, the source file, the processor and the position of the row in headers or a small envelope. Whoever reads the topic later should not need the original file to understand what went wrong.
5. Add a metric and an alert on the dead-letter rate. A few rows is normal. A sudden jump usually means a processor changed its file format, and someone should hear about it quickly.
6. Write a short runbook entry: how to look at the dead-letter topic, how to tell a one-off bad row from a format change, and who to tell on the processor side.

## Design notes

Keep the dead-letter message as close to the original as possible. The point is that a person or a later replay tool can reproduce the failure. Do not reformat the row.

Ordering does not matter for the dead-letter topic, so the key can be the file identifier or the processor, whichever spreads load better. Pick one and write it down in the runbook.

The consumer should not drop a row because it looks like a duplicate of one already in the dead-letter topic. Duplicates in the dead-letter topic are fine; losing rows is not.

If gRPC services upstream expose a status for a settlement file, consider reporting a count of dead-lettered rows there, so that a file is not shown as fully processed when some rows were set aside. This is optional for the first version, but the reconciliation result for that file is incomplete in that case and finance operations should be able to see that.

## Risks

- The dead-letter topic could become a place where problems are hidden. The alert in step 5 is meant to prevent that; do not skip it.
- If the topic is created late, the consumer change cannot be turned on, and the blocking problem stays. This is why the deadline is `2026-12-01` and why the Terraform step comes first.
- Access: the consumers need permission to write to `lark.settlement.dlq`, and the people who investigate failures need permission to read it. Check the ACLs in each environment, not just the topic existence.
- Sensitive data: settlement rows can contain card-related fields. The dead-letter topic holds the same kind of data as the main flow, so it needs the same access controls and retention discipline, not looser ones.

## Open questions

- Retention length: how long do finance operations need to be able to look at a failed row? Ask them before fixing the number in Terraform.
- Whether the replay tool is worth building, or whether manual correction at the source is enough.
- Whether one dead-letter topic is right for all processors, or whether a processor with a very noisy format should be separated later. Start with one.

## Done when

The topic `lark.settlement.dlq` exists in all environments before `2026-12-01`, the consumer sends unparseable rows there and keeps going, the alert is live, and the runbook entry is written. After that, a bad row in a settlement file should no longer stop the consumers of settlement-events.
