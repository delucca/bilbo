---
id: 01JZ8NYTB65BRVY1YR2XEW4XBC
created: 2025-07-03T14:39-03:00
---

# sync-outbox-relay fails with PRECONDITION_FAILED after a queue TTL change

After the queue TTL was changed, sync-outbox-relay stopped starting cleanly and failed with this broker error: `PRECONDITION_FAILED - inequivalent arg 'x-message-ttl'`. The cause is that the queue already existed on the RabbitMQ broker, declared with a different TTL value than the one the new code asked for. RabbitMQ does not let a client redeclare an existing queue with different arguments. The declare call is rejected and the channel that issued it is closed. Changing the number in the relay's configuration or code is not enough on its own, because the broker keeps the old value for as long as the queue exists.

This note is for whoever touches queue arguments next. It covers what the error means, why it shows up in this component, how to recover without losing audit-relevant messages, and how to avoid hitting it again. Read the recovery section before deleting anything. The messages in this queue are part of the audit trail path, and a careless delete is worse than the startup failure.

## What the error says

The broker compares the arguments in a queue.declare request with the arguments the queue was created with. If the queue exists and any of the compared properties differ, the broker answers with a channel-level error, reply code precondition failed. The text names the argument that differs. Here that is the per-queue message time-to-live. The text is `PRECONDITION_FAILED - inequivalent arg 'x-message-ttl'`, and the word inequivalent is the broker's own wording for "the value you sent does not match the stored one".

The error is about equivalence, not about whether the new value is valid. The new TTL could be perfectly reasonable and the broker would still refuse it. It is also not a connection-level failure. The TCP connection and the AMQP connection stay up. Only the channel used for the declare is closed. In a .NET client this surfaces as an exception on the channel with the reply code and the reply text above, and any later call on that channel throws an already-closed exception. That second exception is often what shows up first in logs, so search for the precondition text, not only for the closed-channel message.

## Why it hit sync-outbox-relay

sync-outbox-relay reads pending rows from the SQL Server outbox table and publishes them to RabbitMQ. On startup it declares the exchange and queue topology it needs, so that it can run against a fresh broker without a separate provisioning step. That is convenient, and it is also the trap: the declare in code is the only place the TTL is written down, and it runs against whatever queue is already there.

When the TTL was changed in the relay, nobody changed the queue on the broker. The first start of the new build tried to declare the queue with the new value, the broker compared it to the old value, and the declare was rejected. Because the relay treats a failed topology declare as fatal, it did not publish anything. Rows stayed in the outbox as pending. That is the safe direction to fail in, but it does mean the sync from instrument output to the notebook entries stalls until someone acts.

The environments that already had the queue hit this. A freshly created broker, such as a new developer machine or a clean test environment, declared the queue with the new value without complaint. That difference is why the change looked fine in early testing and failed only where a queue already existed.

## Symptoms to recognise

The visible signs, in the order people usually notice them:

- The relay process starts, logs the topology declare, and then exits or restarts in a loop, depending on how it is hosted.
- The log carries the precondition failure text with the TTL argument named, followed by channel-closed exceptions from the next operation.
- The outbox table grows. Pending row counts rise and the oldest pending row gets older.
- Notebook entries that depend on instrument output look stale to scientists. Compliance officers may notice that audit events are late, not missing.
- On the broker management view, the queue is present with its old arguments and the consumers or publishers from the relay are absent.

If the restart loop is aggressive, the broker log also fills with channel-error lines. That noise is harmless but makes it harder to see anything else. Stop the loop first if it is flooding logs.

## What does not fix it

Several things look like fixes and are not:

- Restarting the relay. The mismatch is in stored broker state. A restart repeats the same declare and gets the same refusal.
- Reverting only the code or config on one instance while others run the new build. That makes behaviour depend on which instance wins the race, and the failing ones keep failing.
- Catching the exception and continuing with a passive declare. A passive declare only checks existence and will succeed, but then the queue silently runs with the old TTL while the code and the docs claim the new one. That is a quiet behaviour mismatch and worse than a loud failure.
- Setting the TTL through a broker policy while the queue also has the argument declared on it. The queue-level argument takes precedence over a policy in the usual precedence rules, so the policy would not change the effective value, and the declare would still conflict.

The broker will not mutate the argument in place. The only ways to get a different per-queue TTL are to create a new queue, or to delete and redeclare the old one.

## Recovery options

There are two sane paths. Pick based on whether the queue holds messages that must not be lost.

The first path is to drain, then delete and let the relay recreate. Stop the relay so nothing new is published. Let consumers finish the queue until it is empty. Confirm in the management view that ready and unacknowledged counts are both zero. Delete the queue. Start the relay with the new TTL, which declares the queue fresh. Nothing is lost, because nothing was in it. This is the cleanest option and should be used whenever the consumers are healthy and the backlog is small enough to clear in a reasonable time.

The second path is to introduce a new queue and migrate. Declare a new queue under a new name with the new TTL, bind it to the same exchange with the same routing, point the relay at the new name, and leave the old queue in place until its consumers have drained it. After it is empty, remove the old queue and its binding. This costs a rename and a short period with two queues, but it never requires deleting a queue that holds data, and it can be done without stopping the pipeline. Prefer it when the queue has a backlog or when consumers are slow or down.

In both cases, the outbox table is the source of truth for what was supposed to be published. Rows are only marked as sent after the broker confirms the publish. So even if something goes wrong on the broker side, the relay can republish from the outbox. That safety depends on the confirm logic being intact, so do not change it in the same release as a topology change.

## Do not delete a queue with unread messages

The delete action in the management view or via the client is immediate. If the queue holds messages that were confirmed to the relay but not yet consumed, deleting it discards them, and the outbox will already consider them sent. That is a gap in the audit trail that no one will see until a reconciliation fails.

Before any delete, look at the target: check the ready count, the unacknowledged count, and whether any consumer is attached. If either count is above zero, do not delete. Use the migration path instead. If you must delete with messages inside, first copy them somewhere durable, and record that in the incident notes so compliance can account for it.

Message expiry is also relevant here. The TTL exists so that stale messages are dropped. Lowering the TTL means messages that were fine before may expire sooner. Raising it means old messages stay longer. Neither change should be made without knowing which messages are allowed to disappear. For this system, an expired message is an audit event that never reached the consumer, so the TTL is a compliance-relevant setting and not just a tuning knob.

## Order of operations for a TTL change

When the TTL really has to change, do the steps in this order and not in the order the code diff suggests:

- Decide the new value and write down why, including what happens to messages near expiry.
- Choose drain-and-recreate or migrate-to-new-queue, based on the current backlog and consumer health.
- Prepare the broker side first: new queue declared and bound, or old queue drained.
- Only then deploy the relay build that declares the new value.
- Watch the first start for the precondition failure. If it appears, the broker side was not finished.
- After the relay is publishing, confirm the outbox pending count falls back to normal.
- Retire the old queue last.

Deploying the code first is exactly how this incident happened. The relay is quick to start and quick to fail, so a wrong order shows up in seconds, but the cost is a stalled sync until someone fixes the broker.

## Making the declare less fragile

A few changes would reduce the chance of repeating this. None are done yet unless stated in the team's tracker.

First, make the queue name carry a revision when the arguments change. A queue whose arguments differ is a different queue as far as the broker is concerned, so giving it a different name says so honestly and makes migration the default instead of an emergency. The cost is that bindings and any dashboards referencing the name must follow.

Second, move the TTL out of the per-queue argument and into a broker policy, which can be changed on a live queue without redeclaring it. That only works if the code stops passing the argument at declare time. Mixing both is the failure described above. This option changes where the setting lives, so the operations side must own it and document it, and the relay should not assume a value.

Third, make the startup failure message more useful. The relay should catch the precondition failure, log that the existing queue differs from the configured value, name the queue, and state the two recovery paths, then exit. Right now people have to recognise the broker text. A clear message saves time at a bad moment.

Fourth, consider not declaring topology from the application at all. A separate provisioning step run by whoever owns the broker would make the mismatch visible at deploy time. The downside is one more moving part for developers, who currently get a working queue for free.

## Configuration notes

The TTL value lives in the relay's configuration and is read at startup, then passed into the queue declaration. Environments differ in what they have set, which is part of why the problem is uneven across them. Before a change, list the effective value in each environment and compare it with what the broker reports for the queue. If they already differ somewhere, the next restart there will fail the same way, even without a code change, for example after a configuration cleanup.

Other queue arguments have the same property. The broker applies the same equivalence check to the queue's durability, exclusivity, auto-delete flag and its other arguments such as dead-letter settings and length limits. So the lesson is general: any change to what a queue is declared with needs a broker-side step. The TTL is just the one that bit us.

Keep the exchange in mind too. Exchanges have the same rule for their type and flags. A change there produces a different precondition message, but the recovery logic is the same.

## Effects on the audit trail

The product exists to keep an audit trail, so the incident has to be judged by what it did to that trail and not only by uptime. During the stall, nothing was lost from the outbox, and the outbox rows carry their original timestamps, so the ordering and the time of the underlying events are preserved. What changes is delivery time. Downstream consumers see the events late, in a burst, when the relay resumes.

Compliance officers should be told when a stall happened and how long, so that late events are not mistaken for tampering or backdating. If the recovery involved deleting or migrating a queue, say so in the same message. If any messages expired because of the old or new TTL during the stall, those events need to be republished from the outbox, since consumers never saw them. Check the outbox for rows marked as sent whose messages did not reach the consumer side, and replay them according to the usual replay procedure.

Idempotence matters in replay. Consumers should tolerate seeing an event twice, since a republish after a partial recovery can duplicate. If a consumer does not deduplicate, hold the replay and ask its owner first.

## Related behaviour to remember

Some neighbouring facts that came up while sorting this out:

- The relay uses publisher confirms. A message is only treated as handed over once the broker confirms it, and only then is the outbox row marked sent.
- A failed declare happens before any publishing, so the stall is clean: no half-published batch.
- Azure Blob Storage is used for larger instrument payloads. The messages carry references, not the bulk data, so a queue stall does not lose blobs, and a queue purge does not delete them. Orphaned blobs may remain if messages that referenced them are dropped, which is another reason not to purge casually.
- SQL Server holds the outbox, and its pending rows are the recovery source. Do not clean up the outbox table during an incident.
- Other services read from the same broker. A queue deletion or rename can affect them if they share bindings, so check bindings before changing anything.

## Checklist for the next person

When you see the precondition failure on startup, work through this:

- Confirm the error text matches `PRECONDITION_FAILED - inequivalent arg 'x-message-ttl'` and that it names the TTL argument, not another one.
- Find which queue it refers to and read its current arguments on the broker.
- Compare with the value the relay is configured to declare. Decide which one is intended.
- If the broker value is intended, revert the relay configuration to match and restart. That is the quickest fix and loses nothing.
- If the new value is intended, check the queue backlog and consumers, then choose drain-and-recreate or migrate.
- Never delete a queue that has ready or unacknowledged messages.
- After recovery, watch the outbox pending count return to normal and tell compliance about the delay.
- Write down the final value and where it is set, so the next change starts from a known state.

## Open questions

A few things are still not settled and should be decided before the next topology change. Who owns the broker side of queue arguments, the relay team or operations? Should the TTL be a policy rather than a declared argument? Is a revisioned queue name acceptable to the other services that bind to the exchange? And should the relay refuse to start on a mismatch, as it does now, or start in a degraded mode that keeps publishing to the existing queue while warning loudly? The current fail-fast behaviour is safe for the audit trail, since it never publishes under settings nobody agreed to, so any change there needs compliance input before it goes in.
