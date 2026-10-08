---
id: 01K6T23JAJK6ZEJSQ54VJ9WJZQ
created: 2025-10-05T08:28-03:00
---

# sync-outbox-relay: queue type research (quorum vs classic)

Notes on whether sync-outbox-relay should publish to RabbitMQ quorum queues or stay on classic queues. Short version: quorum queues cost us publish latency, about 18% more than classic in our testing, so the choice is durability against speed. Internally the component is also called pigeonpost; the codename shows up in older docs and chat, so search for both names.

## Context

sync-outbox-relay reads pending rows from the SQL Server outbox table and publishes them to RabbitMQ. Downstream consumers apply the entries to the electronic notebook and to the audit trail. Instrument output that lands in Azure Blob Storage is referenced from those messages, not carried in them.

## Codename

pigeonpost is the internal codename of sync-outbox-relay. Same service, same repo, same deployment. Use sync-outbox-relay in new writing and keep pigeonpost only when quoting old material.

## Question

Do quorum queues justify their cost for the relay's publish path? Compliance officers care that no audit event is lost. Research scientists care that entries show up quickly after an instrument run finishes.

## What was tested

Same relay build, same outbox backlog, same message sizes, publishing to a classic queue and then to a quorum queue. Publisher confirms were on in both cases. Consumers were kept identical and not the bottleneck.

## Result

Quorum queues add 18% publish latency for sync-outbox-relay compared with classic queues. That is the confirm round trip as seen by the relay, not end-to-end time to the notebook UI.

## Why quorum is slower

A quorum queue replicates each message to a majority of nodes through Raft before confirming. Classic queues confirm after a single node accepts the message. The extra hop is the whole cost, so it will not shrink by tuning the relay itself.

## What we get for it

Quorum queues survive a node loss without dropping confirmed messages. Classic queues, unless mirrored, can lose messages if their node dies. For an audit trail, that is the property that matters.

## Caveats on the measurement

The test ran on a small cluster in a non-production environment. Network latency between nodes will move the number. Treat 18% as a ballpark for planning, not a promise. Retest if the cluster topology changes.

## Effect on the relay

Batch size and the polling interval of the outbox matter more to the user-visible delay than the queue type does. A modest rise in confirm latency is mostly absorbed when the relay publishes in batches and waits for confirms together.

## Effect on the outbox

Rows stay in the outbox until the broker confirms. Slower confirms mean rows stay marked pending a little longer, which slightly raises lock time on those rows in SQL Server. Not seen as a problem yet.

## Ordering

Neither queue type changes our ordering assumptions. Per-entry ordering is handled by the key we publish with and by consumer logic, not by queue choice.

## Recommendation

Use quorum queues for audit-bearing traffic. Keep classic queues only for low-value, rebuildable traffic if any exists. Accept the 18% cost there.

## Alternatives considered

Mirrored classic queues were not tested and are the older approach to the same durability goal. Streams were not evaluated either. Both are open items if the latency cost becomes a real complaint.

## Open questions

Does the cost grow under load with large instrument payload references? Does consumer-side latency change at all? We have no data on either.

## What to check before changing anything

Confirm the queue declaration arguments, since a queue's type cannot be changed in place. Switching means declaring new queues and draining the old ones.

## Migration notes

Drain the old queue, point the relay at the new queue, then remove the old one once empty. Watch the outbox backlog while doing it so nothing is stranded.

## Follow-ups

Rerun the comparison on a production-like cluster. Record the numbers next to the 18% figure here so the two can be compared.
