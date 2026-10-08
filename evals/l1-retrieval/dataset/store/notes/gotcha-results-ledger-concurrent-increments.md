---
id: 01KRWGVRP61BW8FK34ADCS2Y1H
created: 2026-05-18T00:07-03:00
---

# results-ledger concurrent increments need a retry loop

Concurrent increments on the same results-ledger row fail with `restart transaction: TransactionRetryWithProtoRefreshError`, so writers must wrap them in a retry loop. This is the main trap in results-ledger. A single writer never sees it. It shows up when several writers hit the same row at once, which is what a busy live poll does. Anyone adding a new write path to results-ledger has to wrap the increment in a retry loop. Without the loop, votes are lost or the request crashes.

The component used to be called `tallytbl`. It is called `results-ledger` now. Old branches, old dashboards, old chat logs and old runbooks may still say `tallytbl`. It is the same thing. Search for both names when you look for history, and use `results-ledger` in anything new.

## What happens

Results-ledger keeps the running totals for live polls and for Q&A upvotes. Each option or question has a row, and every vote increments a counter on that row. CockroachDB runs these writes as serializable transactions. When two transactions read and then update the same row at nearly the same time, the database cannot order them safely. It aborts one of them and returns this error:

`restart transaction: TransactionRetryWithProtoRefreshError`

The error does not mean the data is corrupt, and it does not mean the database is down. It means this attempt lost a race, and the client is expected to run the whole transaction again. The database tells the client to restart. If nothing restarts it, the error goes up the stack as a failure.

The chance of hitting it grows with contention. One popular option in a large event gets many votes in a very short time, all landing on one row. That is the worst case for results-ledger. A quiet poll with spread-out votes may never show the error in testing. Then the first big event shows it right away. A passing test on a small poll proves nothing here.

## What the writers must do

Every code path that increments a results-ledger row needs a retry loop around the full transaction. Rules I follow:

- Retry the whole transaction, not only the last statement. The read and the write belong to one attempt. Rerunning only the failed statement gives wrong totals or hits the same abort again.
- Retry only on this class of error. Match on the restart error shown above. Other errors, such as a bad argument, a lost connection or a permission problem, must not be retried blindly. They would loop until the cap and hide the real cause.
- Put a cap on attempts, and back off between attempts with a little random jitter. Without jitter, the writers that collided will wake up together and collide again.
- Keep the transaction body free of side effects. Do not broadcast over the WebSocket, send mail, or write to another store inside it. Body code runs again on retry, so a side effect inside it fires more than once. Do the broadcast after the commit succeeds.
- Keep the transaction short. A long transaction holds its read timestamp longer and raises the chance of a restart.
- When the cap is reached, return a clear error to the caller and log it. Do not swallow it. A dropped vote that nobody knows about is worse than a visible failure.

Shape of the loop, in pseudo-Elixir:

```elixir
# results-ledger increment: retry on restart errors only
def increment(row) do
  with_retry(fn ->
    # read and update the row inside one transaction
    # a failure shaped like:
    # restart transaction: TransactionRetryWithProtoRefreshError
    # triggers another attempt of the whole function
  end)
end
```

The real helper should live in one place and every results-ledger writer should call it. I would rather not have each caller write its own loop. Copies drift, and one of them will forget the cap or the match on the error.

## Why it is easy to miss

The Phoenix side hides it well. A vote arrives over a WebSocket, a channel handler calls the write, and the process may simply crash and restart on the error. To the user it looks like one vote that did not count, or a brief reconnect. Nothing in the UI says why. The Next.js front end then shows a total that lags behind the real one, and people suspect the front end first. When counts look low during a busy event, check the server logs for the restart error before touching the client.

Another way to miss it is to treat the write as safe because it is a plain increment. An increment still reads the old value and writes a new one. Under serializable isolation, two of them on one row conflict like any other read-modify-write. It is not an atomic operation that skips conflict detection.

A third way: a test harness that runs votes one after another. That never makes the conflict. To reproduce the error, run many writers in parallel against one row. Use a load test with a single hot option, not many options with few votes each.

## Things I would check when something looks wrong

- Low or lagging totals in a busy poll. Look for the restart error in the logs, and check whether the writer in question goes through the shared retry helper.
- A new write path that skips the helper. Review it for the whole-transaction retry, the error match, the cap and the jitter.
- Duplicate side effects after a retry, such as a double broadcast or a double audit entry. That points to a side effect inside the transaction body.
- Retries that never end. Check that the match is on the restart error only and that the cap exists.
- Latency spikes on one hot row. Retries add delay. If a single row is too hot, the answer is a design change, for example spreading the counter across several rows and summing on read. That is a bigger change and needs its own note. Do not paper over it with a higher retry cap.

## Naming and history

The old name `tallytbl` still appears in some places. If you find it in a migration, a script, a log line or a doc, it refers to `results-ledger`. When you edit such a place anyway, update the name. Do not rename things only for the sake of it if the change risks a data move, since the rename of the component is already done. In notes and commit messages use `results-ledger`.

When reading old discussions about vote loss, keep in mind that some of them were written under the name `tallytbl` and may describe this same restart error in other words, for example as "transaction aborted" or "retry error". Treat them as the same problem unless the details clearly differ.

## Related

Moderation actions also touch live poll state, and the appeal flow has its own timing traps. See [[moderation-gate-roll-appeal]]. It is a different subject from this one, but the same habit applies there: do not assume a write succeeded because the call returned in a quiet test.

## Short version

Concurrent increments on one results-ledger row fail with `restart transaction: TransactionRetryWithProtoRefreshError`. Writers must wrap them in a retry loop that reruns the whole transaction, retries only this error, has a cap with jitter, and keeps side effects out of the body. `tallytbl` is the old name of results-ledger, and the component is called results-ledger now.
