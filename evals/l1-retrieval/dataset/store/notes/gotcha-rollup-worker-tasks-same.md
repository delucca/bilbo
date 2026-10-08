---
id: 01KKSC2JY92B870K0ZDVV6EVTR
created: 2026-03-15T15:28-03:00
---

# rollup-worker deadlock when two tasks hit the same class

If two rollup-worker tasks for the same class run at the same time, one of them can die with `django.db.utils.OperationalError: deadlock detected`. The fix is to take the row locks with `select_for_update(skip_locked=True)`, so a task that finds rows already locked skips them and does not wait on them. This note covers what the failure looks like, why it happens, what we changed, and what to avoid when touching this code later.

## Symptom

The error shows up in the Celery logs for rollup-worker. The traceback ends in `django.db.utils.OperationalError: deadlock detected`. The database picks a victim and aborts one of the two transactions. The other one carries on and usually finishes fine. So the visible result is one failed task and one good task, both for the same class.

Things that make it easy to misread:

- It is intermittent. Most days nothing happens, and then a burst of failures arrives together.
- It clusters around the moments teachers submit or correct marks for a whole class. A single save from a teacher can cause more than one rollup to be queued for that class, and those land on workers at about the same time.
- The failed task looks like a plain application error. Nothing in the message points at another task. You have to look at the logs of the other task for the same class to see the overlap.
- Retrying the failed task usually works, which hides the problem. Auto-retry made the failures look harmless for a while, but it only papers over the collision. Under load the retry can collide again.

The progress numbers that teachers see come from the rollups. When a rollup fails and is not retried, a class can show stale progress against curriculum standards until the next rollup for it succeeds. The next-exercise suggestions that depend on those numbers can be stale too. That is the user-visible cost, and it is why we did not just ignore the log noise.

## Why it happens

A rollup-worker task takes a class, reads the per-student results for that class, and writes aggregated progress rows against standards. To keep the aggregates consistent, it locks the rows it is going to update inside a transaction, with the usual row-level lock from Django's queryset locking.

The trouble is that two tasks for the same class lock overlapping sets of rows. Each task walks the rows in whatever order its query returns them, and nothing guarantees that order is the same between the two tasks. Task A locks some rows and then asks for a row that task B already holds. Task B, meanwhile, has asked for a row that task A holds. Neither can proceed. The database sees the cycle and aborts one transaction with the deadlock error.

Three details made this more likely than it sounds:

1. The rows are not locked all at once. Locks are taken as the query is evaluated and as updates run, so a task holds some locks while it waits for others.
2. Two tasks for the same class are not an exotic case. The same class can be queued twice because several events (new results, a correction, a scheduled refresh) each enqueue a rollup, and Celery is happy to run both in parallel across workers.
3. The rows are shared between students and standards. Even when two tasks are logically about different students, they touch common aggregate rows for the class, so they collide anyway.

It is a classic lock-ordering deadlock. The real cause is concurrency on the same class, and it would still be there with one worker process per host, because there are several worker processes and they can pick up both tasks.

## The fix

We changed the locking read in the rollup so that it uses `select_for_update(skip_locked=True)`. With that option, rows already locked by another transaction are left out of the result instead of making this transaction wait. A task that arrives second therefore does not block on the first task's rows, and without blocking there is no cycle, so no deadlock.

A sketch of the shape, not the real code:

```python
# inside the rollup task, within a transaction
rows = class_rows.select_for_update(skip_locked=True)
```

Be clear about what this does and does not do:

- It removes the deadlock. Nobody waits, so nobody waits in a circle.
- It does not make the second task redo the skipped work. The rows it skipped are being handled by the first task, which is the point. If the first task read its inputs before the new data arrived, the skipped rows could miss the latest results until another rollup runs. See the next section.
- It only works inside a transaction. Outside one, a locking read is meaningless in Django and the database will not hold the locks past the statement. The rollup already runs in a transaction; keep it that way.
- It depends on database support. The database we use for the Django models supports skipping locked rows. If someone ever points rollup-worker at a backend that does not, Django will raise an error at query time rather than quietly ignoring the option, so it will not fail silently.

## Consequences of skipping rows

Skipping is a trade. The task that skips does less than it would have done if it had waited. This is acceptable only because a later rollup for the same class will pick up whatever was missed. We rely on that, so it is worth writing down.

What we rely on:

- Rollups are idempotent. Running one again for the same class gives the same result as running it once, given the same inputs. That is what makes it safe for a task to skip rows and for another task to cover them.
- Rollups are triggered often enough. Because several events enqueue them, the chance that skipped rows stay stale for long is small. If we ever reduce how often rollups are queued, we have to revisit this.
- A rollup that does nothing is a valid outcome. If every row for the class is locked by another task, the second task gets an empty set and finishes without writing. That should not be logged as an error. If you add monitoring, treat an empty result under contention as normal.

What can still go wrong:

- A task skips rows because another task holds them, and the other task then fails and rolls back. The rows are then unlocked and unchanged, and nobody has processed them. They wait for the next rollup. This is the main hole. Retries on the failing task cover most of it, and any later rollup for the class covers the rest.
- Aggregates can be briefly inconsistent between two rollups when one of them skipped part of the class. Teachers might see a number that is one rollup behind. This is the same staleness as before the fix, just with fewer failed tasks.
- If someone adds a check that expects every row of a class to have been touched by every rollup, it will now fail under contention. Do not add that check.

## What not to do

A few tempting changes that we looked at and rejected, so nobody has to rediscover why:

- Do not just add more retries and call it fixed. It lowers the error count but leaves the collision in place, and it adds load exactly when contention is highest.
- Do not serialise all rollups for a class with a global lock outside the database unless you are ready to deal with stuck locks. A lock held by a worker that dies must expire, and picking the expiry is its own source of bugs. Row locks with skipping are simpler and the database cleans them up when the transaction ends.
- Do not drop the lock and rely on optimistic updates. The aggregate rows are updated from computed values, and last-writer-wins would give wrong progress numbers, which is worse than stale ones.
- Do not remove `skip_locked` later because it looks like an odd option. Without it the deadlock comes back, and the symptom is the same intermittent `django.db.utils.OperationalError: deadlock detected` that started this note.
- Do not make the rollup lock rows in a different order in some code paths and expect it to be fine. If you ever move away from skipping and back to waiting, every path must lock in one agreed order, and it must be enforced in one place.
- Do not widen the transaction to include things like Elasticsearch indexing or calls out to the scikit-learn suggestion step. Long transactions hold locks longer, and longer holds mean more skipped rows and more collisions. Do those steps after the transaction commits.

## How to check it still holds

There is no cheap way to prove the absence of a race, but there are checks that catch regressions.

In tests:

- Write a test that starts two rollups for the same class against a real database, not a mock, and runs them concurrently, for example from two threads each with its own connection. Use a transactional test case that allows real commits, since the default test wrapper puts everything in one transaction and hides the locking behaviour.
- Assert that neither raises the deadlock error and that the final aggregates are correct after one more rollup has run. The "after one more rollup" part matters because of the skipping behaviour above.
- Keep the test deterministic by forcing the overlap, for example by pausing the first task after it takes its locks and before it commits, then running the second.

In production:

- Search the Celery logs for the deadlock message after any change to rollup-worker. After the fix it should not appear at all. If it appears again, someone has added a second locking path that does not skip, or a lock is being taken outside the guarded query.
- Watch for classes whose progress looks stale for a long time. That would point at the hole described above, where skipped rows are never picked up because nothing re-enqueues a rollup.
- When contention is high, expect some tasks to finish quickly with little or no work. That is the fix doing its job.

## Where else this could bite

The same pattern can appear anywhere two Celery tasks update shared per-class rows. If another worker is added that writes aggregates for a class, or if a management command does a bulk recompute while the regular workers are running, you can get the same cycle. Check how those paths lock before shipping them. A bulk recompute is the likeliest culprit, since it touches many rows for many classes at once and runs for a long time.

If you see a deadlock error from a different component, do not assume the same fix applies. Skipping locked rows is right here because rollups are repeatable and a later run covers the gap. It is wrong for work that must happen exactly once for every row, such as anything that sends a notification or consumes a queue of items. For those, ordering locks or restructuring the work is the better path.

## Quick reference

- Error: `django.db.utils.OperationalError: deadlock detected`, raised in a rollup-worker task when two tasks for the same class overlap.
- Cause: both tasks lock overlapping rows in different orders and wait on each other.
- Fix: take the locking read with `select_for_update(skip_locked=True)` inside the transaction.
- Price: the task that loses the race may skip rows, and a later rollup for that class covers them.
- Do not: add retries as the only remedy, widen the transaction, or remove the skip option.
