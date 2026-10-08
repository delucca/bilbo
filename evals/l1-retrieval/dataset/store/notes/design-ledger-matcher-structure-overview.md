---
id: 01JWK2ZPRAF16TP70AHSK0QB4N
created: 2025-05-31T08:52-03:00
---

# ledger-matcher: overall structure

This is a working note on how `ledger-matcher` is put together in Ledgerlark. It is written from memory of the design, not from a fresh read of the code, so check details against the source before relying on them. It stays general on purpose: no tuned values, no settings.

`ledger-matcher` is the Go service that takes parsed card-processor settlement records and internal ledger entries and decides, record by record, whether they agree. Anything that does not agree is flagged for a finance operations reviewer. Everything else in Ledgerlark either feeds it or reads what it produces.

## Place in the system

Upstream, settlement files are ingested and normalized by other components. Downstream, a review surface lets finance ops people look at flagged items. `ledger-matcher` sits in the middle and owns the matching logic and the match results. It does not parse raw processor files and it does not own the review UI.

## Inputs

Two streams arrive over Apache Kafka. One carries normalized settlement records. The other carries ledger entries, or changes to them, from the internal ledger. The matcher treats both as append-only facts and never edits either source.

## Outputs

The matcher writes match results and mismatch flags to PostgreSQL, and publishes events on Kafka so other components can react. A gRPC API exposes the results for reads and for a small number of review actions.

## Process layout

One Go binary, run as several replicas. Each replica is a set of goroutines wired together by channels: consumers, a matching stage, a writer, and the gRPC server. There is no shared in-memory state between replicas; coordination goes through Kafka partitioning and the database.

## Consumers

Each consumer reads one topic, decodes messages, and hands them to the matching stage. Offsets are committed only after the result of handling a message is durable in PostgreSQL. A crash means reprocessing, so everything downstream has to tolerate seeing the same message twice.

## Partitioning and keys

Messages are keyed so that a settlement record and the ledger entries it should match land on the same partition, and so on the same replica. That lets the matching stage work on one key without cross-replica locking. The key choice is the most load-bearing part of the design; changing it means a replay.

## Matching stage

For each key the stage holds the candidates seen so far from both sides and tries to pair them. It applies a sequence of rules, starting with the strictest comparison and relaxing in steps. A pair that satisfies a rule is recorded as matched, along with which rule matched it.

## Rule set

Rules compare identifiers, amounts, currency, and timing windows. They are plain Go functions behind a small interface, so a rule can be added or reordered without touching the consumers. Rules are pure: given the same candidates they give the same answer.

## Unmatched and partial cases

A record with no counterpart is not flagged at once, since the other side may simply be late. It waits in a pending state until a waiting period passes, then it becomes a flagged mismatch. Partial matches, such as one settlement covering several ledger entries or the reverse, are handled as groups and recorded with all their members.

## Mismatch flags

A flag carries a reason category, the records involved, and the rule outcome that led to it. The categories are meant to be few and stable so reviewers can filter on them. Flags are state, not events: a later arrival can resolve a flag automatically.

## Storage layout

PostgreSQL holds the settlement and ledger snapshots the matcher needs, the match results, the pending items, and the flags. Writes for one handled message go in one transaction. Uniqueness constraints on the natural keys make replays harmless.

## Idempotency

Reprocessing is expected, so every write is an upsert keyed on identifiers from the input. Event publication follows the database write, and consumers of those events should also tolerate duplicates.

## gRPC API

The API is read-heavy: list flags, fetch a match with its members, and fetch the history of a flag. Writes are limited to review actions such as acknowledging or resolving a flag with a note. The service definitions live with the other Ledgerlark protos.

## Review actions

A reviewer action changes flag state in PostgreSQL and emits an event. The matcher does not undo a reviewer's resolution when new data arrives; it records the new data and marks the flag for another look instead.

## Deployment

Infrastructure is described in Terraform: the service runtime, the database, and the topic definitions it relies on. Configuration is supplied by the environment. Schema migrations are applied separately from the service rollout, and changes are written to work with the old and new binary at once.

## Failure behavior

If PostgreSQL is unavailable, consumers stop committing offsets and back off, and lag builds up in Kafka. If Kafka is unavailable, the API keeps serving reads. Malformed messages go to a dead-letter topic with enough context to be replayed after a fix.

## Observability

The service exposes metrics on consumer lag, pending item age, match rate by rule, and flag volume by category. Logs carry the message key so one item can be followed through the stages. The pending age and the match rate by rule are the first things to look at when results seem off.

## Testing approach

Rules are tested as pure functions with table cases. The matching stage is tested with recorded streams replayed in order and in shuffled order, since arrival order differs in practice. Integration tests run against a real PostgreSQL instance.

## Sketch of the flow

```
settlement topic ─┐
                  ├─> consumer -> matching stage -> writer -> PostgreSQL
ledger topic ─────┘                                   └────> Kafka events
                                       gRPC API <──── PostgreSQL
```

## Known soft spots

- Key choice ties together partitioning and correctness, so it is hard to change.
- The waiting period before flagging is a tradeoff between noise and delay; reviewers have views on it.
- Group matching is the most complex code and the most likely place for bugs.
- Replay after a rule change can rewrite many flags, so reviewer state needs care.

## Open questions

Whether pending items should live in the database only or also in a cache is undecided. How to version rules, so a result can say which rule set produced it, also needs a clear answer before rules change much.
