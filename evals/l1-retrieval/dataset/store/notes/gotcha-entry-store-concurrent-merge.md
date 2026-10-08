---
id: 01KV131JRMNYTQPKFCN79719R2
created: 2026-06-13T15:13-03:00
---

# entry-store-schema: concurrent MERGE deadlocks (Msg 1205)

Concurrent MERGE statements against entry-store-schema fail with Msg 1205: Transaction (Process ID 57) was deadlocked on lock resources. The fix is to add the hints UPDLOCK, HOLDLOCK to the MERGE target. Without those hints, two sessions that run the same MERGE at the same time can each take a shared lock while checking for a match, and then each ask for an exclusive lock on the same resource. SQL Server picks one session as the victim and rolls it back, and that is the Msg 1205 the caller sees. With UPDLOCK, HOLDLOCK on the target, the deadlock stops.

This note is for anyone touching writes to entry-store-schema, whether that is the sync worker, a backfill job, or a one-off script. If you see Msg 1205 in a log, start here.

## The exact failure

The full text of the error, as it shows up in the worker log and in the SQL Server error log, is:

Msg 1205: Transaction (Process ID 57) was deadlocked on lock resources

The rest of the message tells you to rerun the transaction. Do not treat that as the fix. A rerun on its own only hides the problem and adds load, because the same two statements will often collide again when they are retried at the same moment.

Facts to keep straight:

- The error is Msg 1205. It is raised on the victim session only. The other session finishes normally and its caller never knows anything went wrong.
- The Process ID in the text (57 in the case that started this note) is just the session that lost. It is not a fixed id and it will differ from run to run. Do not filter or alert on that value.
- The failing statement is a MERGE against entry-store-schema, and it fails only when another MERGE (or a similar write on the same rows) runs at the same time. A single worker running alone never shows it.
- The fix that worked is adding the hints UPDLOCK, HOLDLOCK on the MERGE target.

If a question comes up later about what fixes concurrent MERGE deadlocks in entry-store-schema, the answer is: add UPDLOCK, HOLDLOCK to the target of the MERGE, and the Msg 1205 stops.

## How it shows up in LabNotebook Sync

LabNotebook Sync takes electronic lab notebook entries and instrument output and keeps them in step. Messages arrive over RabbitMQ, the worker reads them, and the worker writes entry records into SQL Server. Raw instrument files go to Azure Blob Storage, and only the pointers and metadata live in the database. The audit trail is written in the same unit of work as the entry change, so a failed entry write also loses the audit row, and the whole message goes back to the queue.

That shape is what makes the deadlock easy to hit. Several consumers pull from the same queue. When an instrument produces a burst of output, several messages that touch the same notebook entry land on different consumers within a very short window. Each consumer runs the same MERGE to insert the entry row if it is new, or update it if it exists. Two of them hit the same key at nearly the same time, and one gets Msg 1205.

What you see from the outside:

- Messages go back to RabbitMQ and are redelivered. Redelivery usually succeeds, so the problem looks like noise at first.
- The redelivery count on some messages goes up during bursts, then drops again.
- Compliance officers may notice a delay in an entry showing up, but nothing is lost, since the transaction rolls back cleanly.
- The audit trail stays consistent. A rolled back transaction leaves no half-written audit row. This is worth saying to a compliance reviewer who asks, because the rollback is the safe outcome here. The cost is latency and wasted work, not bad data.

## Why it deadlocks

A MERGE without lock hints runs as a read followed by a write. First it looks for a match on the join condition. To do that it takes shared locks, and those are released or downgraded as the scan moves on. Then, depending on the result, it inserts or updates, which needs exclusive or update locks on the rows or key ranges it touches.

Now put two sessions through that at the same time, both aimed at the same key:

- Session A reads the key and finds no match, or finds the row, and holds a shared lock.
- Session B does the same and also holds a shared lock. Shared locks do not block each other, so both proceed.
- Session A now wants to write and asks for an exclusive lock. It has to wait for B to release its shared lock.
- Session B now wants to write and asks for an exclusive lock. It has to wait for A.

Each waits on the other and neither can move. SQL Server's deadlock monitor sees the cycle, picks a victim, rolls it back, and raises Msg 1205 on it. This is a conversion deadlock: both sessions already hold a lock that is too weak for what they are about to do.

The insert path has its own version of this. When the MERGE finds no match and goes to insert, two sessions can both see that the key is absent, and both then try to insert it. Without a range lock held over the check, nothing orders them. In the best case one fails on a key violation; in the worse case they deadlock on the index pages. Holding the range closes that window.

This is not a bug in the data model. The tables themselves are fine. It is a property of how MERGE locks by default, and it is well known in the SQL Server world, but it is easy to forget when writing a new upsert.

## The fix: UPDLOCK, HOLDLOCK

Put the hints UPDLOCK, HOLDLOCK on the target table of the MERGE.

- UPDLOCK makes the read phase take update locks instead of shared locks. Only one session can hold an update lock on a given row or key at a time, so the second session waits at the read instead of getting through to the write and deadlocking. This removes the conversion deadlock.
- HOLDLOCK is the same as serializable isolation for that table in that statement. It keeps the key range locked until the transaction ends, so the "not found, now insert" step cannot be raced by another session inserting the same key in between.

Together they make the check and the write one atomic step from the point of view of other writers. The second session waits for the first to commit, then sees the committed row and takes the update path. No deadlock, no duplicate.

Shape of the statement, with placeholders for the real names (these are not real table or column names):

```sql
MERGE <target table in entry-store-schema> WITH (UPDLOCK, HOLDLOCK) AS t
USING <source rows> AS s
    ON t.<key column> = s.<key column>
WHEN MATCHED THEN
    UPDATE SET <columns>
WHEN NOT MATCHED THEN
    INSERT (<columns>) VALUES (<values>);
```

Points on the hints:

- They go on the target of the MERGE, in the WITH clause right after the table name and before the alias. That is where the lock behaviour for the read of the target is set.
- Both hints are needed. UPDLOCK without HOLDLOCK fixes the conversion deadlock but leaves the insert race. HOLDLOCK without UPDLOCK keeps the range but can still produce a conversion deadlock on the matched path.
- The statement still ends with a semicolon. MERGE requires it.
- Apply the hints on every MERGE that targets the same rows, not only on the one that failed. If one writer is hinted and another is not, the unhinted one can still deadlock against it.

## Where to apply it in the code

The worker is C# on .NET. The MERGE statements live in the data access layer that writes entries and their audit rows, either as inline SQL text or in stored procedures, depending on which part of entry-store-schema is involved. When you search for places to fix, look for any statement that begins with MERGE and targets an entry-store-schema table, in both the C# strings and the SQL scripts that get deployed.

Checklist when you edit or add an upsert:

- Does the statement use MERGE against an entry-store-schema table? Then it needs UPDLOCK, HOLDLOCK on the target.
- Is the MERGE inside a transaction that also writes the audit row? Keep it that way. The hints hold their locks until commit, so keep the transaction short and do no network calls inside it. In particular, do not upload to Azure Blob Storage or publish to RabbitMQ while the transaction is open. Do those before the transaction starts or after it commits.
- Is the statement parameterised? It should be. The hints do not change that.
- Does a test run the statement from several callers at once? See the section on testing below.

Do not widen the fix by changing the isolation level for the whole connection or the whole worker. The hints are scoped to one statement and one table, and that is the point. A connection-wide change would take more locks than needed on unrelated work and slow the whole worker down.

## How to confirm it is this problem

Before applying the fix to a new place, check that the failure is the same one. Signs that it is:

- The error is Msg 1205 and the text says the transaction was deadlocked on lock resources.
- The statement named in the deadlock report is a MERGE against entry-store-schema.
- The deadlock graph shows both sessions as victims-or-owners on the same table or index, each holding a shared or update lock and waiting for an exclusive one.
- It happens during bursts, when several consumers are busy, and not when the system is quiet.

If you get Msg 1205 on a different statement shape, for example two plain UPDATE statements that touch rows in opposite order, this fix does not apply. That is an ordering problem and needs a different change. Do not paste the hints onto an UPDATE and expect the same result.

To see the graph, pull the deadlock report from SQL Server's system health data or from the trace you already have turned on for the environment. Read the two processes, the resources, and the statements. The MERGE text will be visible in the input buffer for each process.

## Testing and rollout

A single-threaded test will never show this. To reproduce, run the same MERGE from several concurrent connections against the same key, in a loop, until one of them returns Msg 1205. Without the hints, this reproduces quickly on a dev database. With UPDLOCK, HOLDLOCK on the target, the same loop should finish with no Msg 1205 and no duplicate rows.

What to check after the change:

- No Msg 1205 from the MERGE under the concurrent loop.
- No duplicate rows for the same key. The key range lock is what guarantees this.
- Throughput under burst is acceptable. Writers to the same key now queue instead of racing, so expect a little waiting on hot entries. Writers to different keys are not affected.
- RabbitMQ redelivery counts during bursts drop back to normal.
- The audit trail still has one row per change, written in the same transaction.

Rollout notes:

- The change is to statement text only. There is no schema change in entry-store-schema, no migration, and no data fix needed. You can roll it back by removing the hints.
- Deploy the worker and any changed stored procedures together, so there is no period where some writers have the hints and others do not. A mixed state still deadlocks, just less often.
- Keep the retry on the consumer side. A retry is still a good safety net for transient SQL errors. It is just not the fix for this one.

## Things not to do

- Do not catch Msg 1205 and swallow it. The write did not happen, and swallowing it loses the entry update and its audit row.
- Do not add a blind retry loop with no limit around the MERGE. It adds load during exactly the bursts that cause the problem.
- Do not drop the audit write to shorten the transaction. The audit trail is the whole reason this system exists, and compliance officers rely on it being written with the entry.
- Do not use only one of the two hints. Both are needed.
- Do not hold the transaction open while calling out to other services. With HOLDLOCK the locks last until commit, so a slow call inside the transaction blocks every other writer on that key.

## Short version for the next person

If you see Msg 1205: Transaction (Process ID 57) was deadlocked on lock resources from concurrent MERGE statements against entry-store-schema, put the hints UPDLOCK, HOLDLOCK on the MERGE target in every MERGE that touches those rows. Keep the transaction short, keep the audit write inside it, and test with concurrent callers on the same key. The Process ID in the message is only the losing session and is not meaningful on its own.
