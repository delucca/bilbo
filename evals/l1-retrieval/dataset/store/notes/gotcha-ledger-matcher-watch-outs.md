---
id: 01JVR57K50TG7PH9C7P6P6Y7GN
created: 2025-05-20T21:52-03:00
---

# ledger-matcher: things to watch when changing it

Notes from working on ledger-matcher. Not a spec, just the places where a change tends to bite. Read this before touching matching rules, the consumer loop, or the queries behind it.

## Matching logic

The matching rules look simple and are not. Settlement files and ledger entries rarely agree on amount, currency, timing and reference in the same way. Small changes to how amounts are compared (rounding, minor units, fees netted out or not) can flip large groups of items between matched and flagged. Before changing a rule, think about what happens to items that were already matched under the old rule. Do they get re-evaluated, or stay as they are? Either answer can be wrong for finance ops.

Partial matches, one-to-many and many-to-one cases are easy to break. Refunds, chargebacks and reversals show up as separate lines and need to pair with the original, not just with anything of the same size. Keep the tie-breaking order deterministic. If two candidates are equally good, the pick must not depend on map iteration order or on row order from the database.

Time handling: processors report in their own time zones and cutoffs. Ledger entries may be posted later than the settlement date. Do not compare timestamps naively, and do not widen a tolerance window without checking what else falls into it.

## Kafka consumption

Messages can be delivered more than once and can arrive out of order across partitions. Anything that writes a match result has to be idempotent. A replay of the same input must produce the same outcome and not a second flag or a second match record.

Be careful with offset commits. Committing before the database write finishes loses work on a crash. Committing after is fine only if the write is safe to repeat. If you change the message shape, remember that old messages may still be sitting in the topic and the matcher has to read them. Changing the partition key changes ordering guarantees that the matching code may quietly rely on.

Rebalances happen in the middle of a batch. Do not keep per-partition state in memory that is not rebuilt after a rebalance.

## PostgreSQL and gRPC

Matching queries run over big tables. A new filter or join that skips an index looks fine on a dev data set and then crawls in production. Check the query plan on realistic volumes. Long transactions that hold locks on ledger or match tables block other writers, so keep them short and batch with care.

Schema changes need to work while the old version is still running, since deploys are rolling. Add first, migrate data, switch code, drop later. Do not rename columns in one step.

For the gRPC surface, only add fields; do not reuse or renumber existing ones. Other services and the review UI depend on the current meaning of status values, so adding a new match status is a breaking change for any client that switches on it. Check who consumes it first.

## Rollout and review

Money-related output is audited. A change that alters which items get flagged should be run against a copy of recent data, with the old and new results diffed, before it ships. Say in the change description what moves between matched and flagged.

Infrastructure for the component is in Terraform. Changes to consumer settings, resources or topic config should go through the same review as code, and not be done by hand in a console. Watch for drift if someone already tweaked something manually.

Logging: do not write card data or full account identifiers to logs while debugging. Use references that finance ops can look up instead.
