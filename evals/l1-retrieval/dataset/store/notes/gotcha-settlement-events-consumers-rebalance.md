---
id: 01JVZ839XEMATW9HSWMAR8Y6XZ
created: 2025-05-23T15:57-03:00
---

# settlement-events: REBALANCE_IN_PROGRESS on offset commit during pod restart

Consumers of settlement-events log `REBALANCE_IN_PROGRESS` on offset commit when a consumer pod restarts during a rebalance, and the batch is reprocessed. This is the thing to remember about this topic. The error looks like a failure, but it is a signal that the group moved under the consumer while it was working. The commit did not land, so the broker still thinks the old position is the current one. The next owner of the partition starts from there and handles the same records again.

This note is for anyone who sees duplicate work after a deploy, a node drain or a crash loop on the settlement-events consumers. It also covers why the duplicates are harmless only as long as the write path stays idempotent, and what to check when it is not.

## Symptom

The visible signs, in the order people usually notice them:

- A burst of error lines from a consumer of settlement-events at the moment it tries to commit offsets. The text includes `REBALANCE_IN_PROGRESS`.
- Around the same time, a pod restart. It can be a rollout, an eviction, an out-of-memory kill or a plain crash. The restart does not have to be the pod that logs the error. A healthy neighbour pod can log it because the restart of another member started the rebalance.
- Afterwards, the same settlement records show up again in the consumer's processing logs. Reconciliation counters for that period go up more than the input volume explains.
- Sometimes the review queue briefly shows the same mismatch twice, or a mismatch that was already resolved reappears for a short while.

None of this means data was lost. It means data was seen more than once. The distinction matters, because the first reaction of most people is to look for missing settlement rows, and the rows are not missing.

## What happens, step by step

The sequence is the same each time, and it is worth having in your head before reading any logs.

1. A consumer in the group fetches a batch from settlement-events and starts processing it. Processing means decoding the record, matching it against internal ledger entries in PostgreSQL, and writing the match result or the mismatch flag.
2. While the batch is in flight, a pod restarts. The group coordinator notices a member leaving or joining and starts a rebalance.
3. The consumer that was processing finishes its batch, or most of it, and tries to commit the offset for what it handled.
4. The coordinator answers that a rebalance is in progress. The commit is refused. The log line carries `REBALANCE_IN_PROGRESS`.
5. The rebalance completes. Partitions are reassigned. Whoever gets the partition resumes from the last committed offset, which is the one from before the refused commit.
6. The whole batch is processed again.

The key point is step 4 followed by step 6. The work in step 1 already had side effects in PostgreSQL. The commit that would have recorded that work was rejected. So the side effects exist and the offset says they do not.

## Why the batch is reprocessed

Offsets are the only memory the broker keeps about how far a group has read. Processing and committing are two separate actions, and nothing makes them atomic. When the commit is refused, the group position stays where it was.

There is no partial credit. Even if the consumer handled most of a batch before the refusal, the position moves only on a successful commit, and a successful commit covers a position, not a list of records. So the safe resume point is the old one, and everything after it is redone.

This is at-least-once delivery working as designed. The design gives up exactly-once in exchange for never skipping a record. For settlement data, skipping is the worse failure, so the trade is right. But it puts a duty on the consumer: every write it makes has to survive being made twice.

It is tempting to think that a retry of the commit would fix it. A retry inside the same consumer generation usually will not, because that consumer may no longer own the partition once the rebalance finishes. After the rebalance, the member that retries could be committing for a partition it has lost, and the coordinator will reject that too. Retrying blindly produces more noise and no progress.

## Why it matters for Ledgerlark

Ledgerlark compares card-processor settlement files with internal ledger entries and flags differences for finance operations teams at online marketplaces. A duplicate in this pipeline is not cosmetic. If a settlement record is applied twice, a total can be wrong, a mismatch can be raised that should not exist, or a mismatch can be cleared that should still be open.

Finance teams read these flags and act on them. Someone may chase a processor over a difference that was only a reprocessing artefact. That costs a person's time and it costs trust in the tool. A tool that cries wolf after every deploy gets ignored, and then the real mismatch gets ignored as well.

So the standard for this component is that a replay must be invisible in the final state. The logs may show the duplicates. The database and the review queue must not.

## How to spot it in logs

Search consumer logs for the error name and look at what comes right before and after. A typical line, reduced to the part that matters:

```
commit offsets failed: REBALANCE_IN_PROGRESS
```

The exact wrapping text depends on the client library and on how the consumer formats errors, so search for the constant, not for the sentence around it.

Things to line up once you have found it:

- The time of the pod restart from the orchestrator events. The error should sit shortly after it, or shortly after another member joined.
- The partitions that were assigned before and after. If they moved to another pod, the replay happened on a different machine than the original attempt. That is why per-pod logs alone can look inconsistent.
- The next successful commit. Between the refused commit and that one, everything is a replay.

If the error appears with no restart nearby, look for a slow batch. A consumer that takes too long to poll can be removed from the group by the coordinator and cause the same chain. The cure for that is different from the cure for a restart, so do not assume the restart story fits every occurrence.

## What not to do

A few reactions that look sensible and are not.

- Do not treat the error as fatal and exit. Exiting causes another restart, which causes another rebalance, which causes the same error on the next pod. This is how a small incident becomes a loop.
- Do not swallow the error silently. The commit failed, and the code after it must know that. If the code carries on as if the position had advanced, in-memory state and broker state drift apart.
- Do not switch to committing before processing to make the error go away. That turns at-least-once into at-most-once, and a crash after the commit loses records. For settlement data this is the wrong direction.
- Do not delete rows from PostgreSQL by hand to undo a replay unless you have confirmed they are real duplicates. If the write path is idempotent there is nothing to undo, and if it is not, deleting by hand without understanding the key is how ledger data gets damaged.
- Do not widen the batch to reduce the number of commits. A bigger batch means a bigger replay when this happens.

## Mitigations that work

None of these removes the error. They make its effect small.

### Idempotent writes

The real defence is in the write path. Every write derived from a settlement-events record should be keyed by something stable that comes from the record itself, and the database should refuse or ignore a second write with the same key. Use the uniqueness machinery in PostgreSQL rather than a check in application code, because a check in code has a window between the read and the write, and two consumers replaying around a rebalance can fall into that window.

The mismatch flags need the same treatment. A flag raised twice should be one flag. A flag that a reviewer already resolved should not be reopened by a replay of the record that first raised it. That second rule is easy to forget and is the one finance users notice first.

### Smaller, bounded batches

Keep the amount of work between commits modest. The cost of this gotcha is proportional to the work done since the last good commit. Smaller batches cost more commit round trips and less replay. Pick the batch size by looking at how long a replay takes against how much the extra commits cost, not by guessing.

### Graceful shutdown

When a pod is told to stop, the consumer should stop fetching, finish or abandon the current batch cleanly, commit what it can while it still owns the partitions, and leave the group on purpose. A consumer that leaves on purpose triggers a rebalance sooner and with less confusion than one that simply disappears and has to be timed out. The termination grace period in the deployment has to be long enough for this to complete. If the orchestrator kills the process before it finishes, you are back to the unclean case.

### Rollout pace

Rolling all consumer pods quickly causes rebalance after rebalance. Rolling them one at a time, and waiting for the group to settle before the next, keeps the number of rebalances and the size of replays down. This is a deployment setting, so it lives with the Terraform and orchestrator configuration, not in the Go code.

## Idempotency in the PostgreSQL write path

This section is the one to reread before changing any consumer code.

The consumer is written in Go and talks to PostgreSQL for ledger lookups and for storing results. Any new write added to the handler for settlement-events has to answer one question: what happens if this exact record is handled again, by a different pod, an hour from now, with the first attempt half applied?

Checks to run on a new write:

- Is there a natural key from the record that identifies it? If so, is that key enforced as unique in the table?
- Does the insert use a conflict clause so that the second attempt is a no-op, or an update that produces the same final row?
- If the handler does several writes for one record, are they in one transaction? A replay that finds the first write done and the second missing should complete the second, and a transaction makes that case rare.
- Do counters and totals get computed from stored rows, or incremented by the handler? Incrementing in the handler is the usual source of totals that drift after a replay. Computing from rows is safe.
- Does anything outbound happen, such as a notification or a message to another service through gRPC? Those are side effects the database cannot deduplicate. They need their own key on the receiving side, or they need to be sent from a place that runs once per stored change rather than once per delivery.

A useful habit is to test the handler by feeding it the same batch twice and comparing the database state to the state after one pass. They should be identical.

## Deploys, Terraform and restarts

Most occurrences come from planned change, not from faults. A deploy replaces pods, and replacing pods moves partitions. If a deploy lands while the consumers are busy, a refused commit is likely.

Points to keep in mind when changing infrastructure that touches this consumer group:

- Changes applied through Terraform that cause a replacement of the consumer workload act like a deploy. Review the plan for anything that recreates the pods or changes their count, and expect a rebalance when it is applied.
- Autoscaling that adds and removes consumers causes rebalances by itself. Scaling on a noisy metric means frequent rebalances and frequent replays. Prefer a slow, damped signal for this group.
- Node maintenance that drains several nodes in a row has the same effect as a fast rollout. Spread drains out where possible.
- A change to broker or topic settings can also trigger group movement. Treat those as deploys for this purpose.

If a replay is going to happen anyway, pick the moment. Applying changes when the settlement files for the day have been fully consumed and traffic is low makes the replay small and the chance of a person noticing smaller still.

## Checking state after an incident

After a restart-heavy period, confirm the system ended up consistent. A short routine:

1. Establish the window: first and last occurrence of `REBALANCE_IN_PROGRESS` in the consumer logs, with the restarts that caused them.
2. Confirm the group has settled: every partition of settlement-events has an owner, and committed positions are advancing again.
3. Compare record counts and totals in PostgreSQL for the window against the source. Duplicates would show up as totals above the source, or as repeated rows where a unique key should have prevented them.
4. Look at the review queue for flags created in the window. Group by the settlement record that raised them. More than one flag per record means a write is not idempotent, and that is a bug to fix, not noise to clean up.
5. Look for flags that a reviewer had already resolved and that came back open. If there are any, the replay path overwrites reviewer decisions and needs a fix.
6. Tell the finance operations contact only if the totals, the flag grouping or the reopened flags turned up something real. A replay that left no trace needs no announcement.

If the flag checks find a problem, record which write path caused it in this note, so the next person starts from there.

## Related behaviours that look similar

Two other things get mixed up with this one.

First, a consumer that is slow rather than restarted. The coordinator can decide a slow member is gone and rebalance without any pod restarting. The log symptom can match. The cause is a handler that takes too long between polls, often because a PostgreSQL query is slow or a gRPC call is waiting. Fixing it means fixing the slow call, not tuning deployments.

Second, duplicates that come from the producer side. If the settlement file importer publishes the same record twice, the consumers will see duplicates with no rebalance at all. The write path defence is the same, but the investigation is on the producing side. Check for the commit error first. If it is absent around the time duplicates appeared, look upstream.

## Open questions

Things nobody has pinned down yet, listed so they are not rediscovered from scratch.

- Whether every write in the settlement-events handler is idempotent today. The handler has grown over time and nobody has done the double-feed test on all of it.
- Whether outbound calls made during handling are deduplicated by their receivers.
- Whether the shutdown path really commits before the process exits under the current grace period, or only does so in the happy case.
- Whether the autoscaling signal for this group is stable enough to avoid rebalances without a deploy.
- Whether alerting should fire on this error at all. Today it is a log line. A rate alert may be more useful than a per-occurrence one, since a single occurrence after a deploy is expected and a sustained stream is not.

When one of these gets answered, update this note and delete the question.
