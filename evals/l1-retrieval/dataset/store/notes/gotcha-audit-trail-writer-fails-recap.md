---
id: 01KSFCMV7B3P5T5BAXBXHTKD3G
created: 2026-05-25T07:59-03:00
---

# audit-trail-writer fails: second pass

Wrote this down because audit-trail-writer failing keeps coming up and I did not go back through what we already had before typing. Treat it as a loose second take on the same trap. If the older note on audit-trail-writer failures says something different, trust whichever one has the more recent evidence behind it and fix the other. Nothing here is a full root-cause write-up. It is what I remember from poking at it, in the order it bit me.

The short version: when audit-trail-writer fails, it usually does not fail loudly where the scientist is looking. The notebook entry looks saved. The instrument output looks received. The gap shows up later, when someone from compliance asks for the trail and part of it is missing or out of order. So the failure is easy to miss for a long time, and that is the actual gotcha. The crash itself is almost secondary.

## What it looks like from outside

The symptoms I have seen, roughly in order of how often:

- Entries exist in the notebook but the matching audit rows are absent, or they exist but arrive well after the entry they describe.
- The queue in RabbitMQ grows and nobody is draining it. The consumer for audit-trail-writer is either not connected or is connected and stuck on one message.
- The same audit event appears more than once, because a message was redelivered after the writer died partway through handling it.
- Attachments or instrument files are referenced in the trail but the blob they point at is not in Azure Blob Storage yet, or the reference was written before the upload finished.
- Sequence gaps. The trail has an ordering requirement, and when the writer skips an event and carries on, the ordering looks fine to a casual reader but is wrong when you compare against the entry history.

None of these throw anything a user sees. A compliance officer would see them only in a review. A research scientist would see them only if they happened to open the history view for an entry that was affected.

Rough flow, so we are talking about the same thing:

```
instrument output -> RabbitMQ -> audit-trail-writer -> SQL Server
                                                   \-> Azure Blob Storage
```

The writer sits in the middle and talks to three things. Any of the three can be the reason it fails, and the logs do not always make it obvious which. That is part of why this keeps eating time.

## Usual suspects

### SQL Server side

The most common cause I would bet on is the database write failing in a way the writer treats as retryable when it is not, or the reverse. Things that fit:

- A timeout under load. The audit table is append-heavy and shares the server with everything else. When a long-running query or a maintenance job holds locks, the writer's insert waits past the configured command timeout and gives up. The usual timeout value is fine for normal traffic and too short for bursts.
- A deadlock between the writer and the notebook application touching related rows. The database picks a victim, and sometimes the victim is the writer. If the writer's retry is not careful, it either drops the event or retries forever.
- A constraint violation from a duplicate. This is the redelivery case: the first attempt actually committed, the acknowledgement never made it back, the message came again, and the second insert hit a uniqueness rule. If that error is treated as a hard failure, the message gets rejected and possibly dead-lettered even though the data is already there. If it is treated as success, fine. Which one it does depends on the code path, and I did not verify every path.
- A schema change that the writer was not rebuilt against. Adding a column on the notebook side without matching the writer's mapping is an easy way to break inserts quietly.

### RabbitMQ side

- The connection drops and the consumer does not recover cleanly. The client library has automatic recovery, but topology recovery and consumer re-registration have been where things went wrong before. The process stays alive, the health check is green, and nothing is being consumed.
- Prefetch set too high, so one slow message holds a large batch of unacknowledged messages hostage. When the writer then fails, all of them go back to the queue at once and get redelivered in a flurry.
- Acknowledging before the work is done. If the ack happens early, a crash loses the event for good, and for an audit trail that is the worst possible outcome. If the ack happens late, we get duplicates instead. Duplicates are the better problem to have, and the design should lean that way. I believe it does, but check before relying on it.
- Dead-letter handling. Messages that exhaust retries go somewhere, and I am not sure anyone looks there. A quiet dead-letter queue is not proof that nothing failed; it may mean nobody wired up the alert.

### Azure Blob Storage side

- Transient storage errors or throttling during upload. The writer retries, and the audit row can end up written first with the blob reference pointing at something that is not there yet.
- Credential or token expiry. If the writer authenticates with something that rotates, a rotation that is not picked up by a running process leads to a stretch of failures that clears itself on restart, which makes it look like a flaky bug instead of a config one.
- Large instrument output files taking longer than the operation timeout allows. The small files all work, which is why this goes unnoticed until a particular instrument sends something bigger than usual.

## Ordering and the write path

The thing I would flag for anyone changing this code: the order of operations inside the writer matters more than it looks. For an audit trail you want the durable record to be the source of truth, and everything else to be derivable from it or at least detectable when missing. What I have seen suggests the writer does some of this in a different order depending on the event type, which is how the blob-reference-before-blob problem got in.

A sane order, as I understand the intent:

1. Receive the message and do not acknowledge it.
2. Upload any referenced file to storage and confirm it landed.
3. Write the audit row to SQL Server in a transaction, with whatever idempotency key the event carries.
4. Only then acknowledge.

If the database write fails, the message goes back and the upload repeats, which should be harmless if the upload is idempotent for the same content. If it is not idempotent, we get orphan blobs, which are annoying but not a compliance problem. Orphan blobs are much better than missing audit rows.

I did not confirm that the code follows exactly this. I read part of it, not all, and some branches handle things differently. Do not take this section as a description of what the code does; take it as what the code should be doing.

## How to tell which dependency is at fault

When the writer is reported as failing, I would go in this order, because it is cheapest first:

- Look at whether the queue is growing. If it is growing and the consumer count is zero, it is a connection or process problem and not a data problem. Restart is a legitimate first move, but write down what you saw before you do it.
- If the consumer is present but the queue is not draining, look for one message being retried repeatedly. A poison message at the head with a low prefetch can block everything behind it.
- If messages drain but rows are missing, look at the dead-letter queue and at the writer's own logs around the missing events. Missing rows with a clean queue means something acknowledged without writing.
- If rows exist but blobs do not, or the other way round, it is the storage path, and the question is which of the two the writer did first.
- Compare against the notebook's own entry history. The entry history is a second record, and a mismatch between it and the audit trail tells you the window in which the writer was unhealthy.

The log lines from the writer are not as consistent as I would like. The same underlying failure can show up under different messages depending on which library raised it. I am deliberately not quoting any here because I do not trust my memory of the wording, and matching on exact text has burned us before when a library update changed it.

## Things that make it worse

- Restarting the writer repeatedly without checking the queue. Each restart redelivers whatever was unacknowledged, and if the cause is a poison message, you just reproduce it.
- Purging the queue to make an alert go away. Do not do this. The messages are audit events. If they are bad, move them somewhere and keep them.
- Fixing the missing rows by hand-inserting them. For a regulated trail, a hand edit is itself something that needs to be in the trail, and the compliance people will want to know who did it and why. If rows have to be backfilled, do it through the writer or through a procedure that records that it was a backfill.
- Assuming a green health check means it is working. The health check, as far as I can tell, reports on the process and not on whether it is consuming and committing.
- Changing retry settings without thinking about duplicates. More retries means more chance of the duplicate case, and the duplicate case only works if the idempotency handling is right.

## Compliance angle

Since the users include compliance officers, a failure in this component is not just an outage. A gap in the audit trail may need to be reported, and it may need an explanation of the window in which it happened and what was affected. So when this breaks, the useful artifacts are not only the fix but also: when it started, when it stopped, which entries fall inside that window, and whether any events were lost or just delayed. I would capture those while it is fresh, because reconstructing them later from logs that have rolled over is painful.

Delayed is acceptable if the original timestamps of the events are preserved and the trail makes clear that the write came later. Lost is not acceptable, and detecting lost events depends on having something to compare against. That is another reason the notebook-side entry history matters.

## What I am not sure about

- Whether the writer treats a duplicate-key error as success. This is the single most important thing to confirm, and I did not.
- Whether prefetch is set to something sensible or left at a default. The usual value may be fine for the volume we have, but it should be a decision, not an accident.
- Whether anything alerts on the dead-letter queue. My guess is that it does not, but I did not look at the monitoring config.
- Whether credential rotation for storage is handled without a restart.
- Whether the order of operations is the same for every event type. I suspect it is not.
- Whether the older note on this subject already covers some of these. If it does, merge them and drop the duplication.

## Next steps if someone picks this up

Read the writer's message handler end to end and write down the actual order of operations per event type. Check the ack placement. Check how it classifies database errors into retry, reject, and treat-as-done. Look at the dead-letter setup and add an alert if there is none. Add a health check that reflects consuming and committing, not just that the process is up. Then add a test that kills the writer between the database commit and the acknowledgement and confirms the redelivered message does not produce a second row or a failure.

Once those are answered, this note should be folded into the main one on audit-trail-writer failures and removed, so there is a single place to look.
