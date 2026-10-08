---
id: 01M35YMTWXPZ30F2TGEQ80K0Z8
created: 2026-09-22T22:38-03:00
---

# state-store: concurrent writers hit SQLITE_BUSY

Two workers writing to the state-store at once can fail with `SQLITE_BUSY: database is locked`. It does not happen on every overlap, so it passes a quick local run and shows up under load, usually when several upgrade jobs finish at about the same moment. This note records what the error means, why it shows up, how to recognise it, and what to do and not do about it.

## Symptom

A worker that is recording progress for an upgrade pull request throws `SQLITE_BUSY: database is locked`. The call that fails is a write: inserting a run record, updating the status of a verification, or saving the result of a targeted test run. Reads almost never fail this way.

What it looks like in practice:

- A job in GitHub Actions goes red, but the dependency upgrade and the tests were fine. Only the bookkeeping step failed.
- The pull request exists and the branch is pushed, but the state-store has no final status for it, or still shows the run as in progress.
- Re-running the same job later passes without any change. That is the strongest hint it is lock contention and not a bug in the upgrade logic.
- The failure clusters. If one job hits it, others started around the same time often hit it too.

The message is the SQLite error passed through by the Node.js driver. Do not assume the text is wrapped or rephrased by our code. Search logs for the exact string `SQLITE_BUSY: database is locked`, not for a paraphrase.

## Why it happens

The state-store is a single SQLite database file. SQLite allows many readers but only one writer at a time. When a second worker tries to start a write while the first still holds the write lock, SQLite waits for a while if a wait is configured, and gives up with `SQLITE_BUSY: database is locked` if the lock is not released in time. With no wait configured it gives up almost at once.

Things that make the window bigger in our setup:

- Several workers are started in parallel by the workflow, each as its own process, all pointing at the same database file.
- A worker opens a transaction early and keeps it open while it does slow work such as calling the GitHub API or waiting on a test process. The lock is held for the whole time, not just for the write itself.
- A transaction that starts as a read and later tries to upgrade to a write can fail immediately, because another writer got in between. This kind of failure can happen even when a wait is configured, because waiting would not help the upgrade succeed.
- Container volumes that are shared between jobs can have slower locking than a local disk, so the lock is held longer than on a laptop.

So the error is not corruption and not a sign that the file is damaged. It is the database telling the second writer to come back later.

## How to reproduce

The easiest way is to start two or more workers against one state-store file and make them write in a tight loop. A small script that opens separate connections, begins a write transaction in each, and sleeps inside the transaction will fail the other connection quickly.

Notes for reproducing:

- Use separate processes, or at least separate connections. Two operations on one connection never conflict with each other, so a single-process test proves nothing.
- Hold the transaction open on purpose. Short writes in a loop may collide only now and then.
- Run it in Docker too if the failure only appears in CI. Behaviour differs between a bind mount on a laptop and a volume in a container, and the lock timing differs with it.
- A test that passes once does not show the problem is gone. Repeat it many times, or add delay inside the transaction, before trusting it.

## What to do about it

Pick the least invasive fix first, and combine them if needed.

1. Keep write transactions short. Do all the slow work before opening the transaction, then open it, write, and commit. Never call out to the network or to a child process while a write lock is held.
2. Configure a wait on the connection so a blocked writer retries inside SQLite for a reasonable time before it fails. Check how the connection is opened in the state-store code and make sure the wait is set in one place for every connection, not per call site.
3. Start write transactions as write transactions from the beginning, so they take the lock up front. This avoids the case where a read-then-write upgrade fails instantly.
4. Add a small retry with backoff and some jitter around the write helper for this specific error. Retry only on this error, not on every failure, and put a limit on attempts so a real problem still surfaces. Jitter matters because workers that failed together will otherwise retry together and collide again.
5. Use the journaling mode that lets readers continue while a writer is active. It does not allow two writers, but it stops readers from being blocked and shortens the time writers wait. Confirm the mode is actually applied on the file that the workers use.
6. If the number of workers keeps growing, funnel writes through one process or a queue so only one component writes at a time. This is a bigger change and should only come when the smaller fixes are not enough.

## What not to do

Some fixes look attractive and make things worse or hide the problem.

- Do not delete the database file or any side files next to it to clear a lock. A leftover lock from a crashed process is released by the operating system when the process dies, so deleting files will not help and can lose data or corrupt the store while another worker is writing.
- Do not swallow the error and continue. The upgrade would look finished while the state-store is missing the record, and the next run might repeat work or open a duplicate pull request.
- Do not raise the wait to a very large value as the only fix. It turns a clear failure into a stuck job, which is harder to notice and burns runner time.
- Do not give each worker its own copy of the file and merge later unless that is a deliberate design change. Copies drift and the merge becomes its own source of bugs.
- Do not assume a retry is safe for non-idempotent writes. Before wrapping a write in a retry, check that doing it twice cannot create two rows or advance a status twice.

## Checking whether a failure was this error

When a job fails and it is unclear why, go through these in order:

- Look in the job log for `SQLITE_BUSY: database is locked`. If it is there, the cause is lock contention, whatever else the log says afterwards.
- Find out which other jobs were running against the same state-store at that time. Overlap in time is the usual explanation.
- Look at what the failing worker was doing when it failed: starting a run, recording a test result, or closing the run out. Failures while closing out are the most damaging because they leave runs looking unfinished.
- Check whether the worker had a transaction open during a slow step. If so, that is the code to fix, even if another worker was the one holding the lock.
- Check the state-store for runs that have no final status. After a failure like this, those rows may need to be reconciled by hand or by a re-run.

If the string is not in the log, do not blame the state-store. Other failures, such as a full disk or a bad path, produce different errors and need different fixes.

## Effects on the rest of PatchPilot

The state-store is what lets PatchPilot know which upgrade pull requests it has already opened and which targeted tests have already passed. When a write is lost to this error, the effects reach beyond that one job.

- A pull request can be opened twice if the record that says it exists was never saved. Platform engineers who maintain many repositories will notice this as duplicate pull requests from the bot.
- Verification results can go missing, so a pull request looks unverified even though its tests passed. Someone may then re-run tests that did not need re-running.
- Scheduling decisions based on past runs, such as skipping a dependency that was recently attempted, can be wrong for a while.
- Noise in the workflow results trains people to ignore red jobs. That is a cost on its own, so fix the cause instead of telling people to re-run.

When reviewing a change that touches the state-store, ask whether it adds a new writer, lengthens a write transaction, or opens a new connection. Any of those can make this error more likely.

## Open questions and follow-ups

Things that are still not settled and worth deciding when someone next works here:

- Whether the wait on the connection is configured the same way everywhere the state-store is opened, including tests and the Docker entrypoint. If there are several places that open the file, they should share one helper.
- Whether any code path holds a transaction open across a network call. A review of the write paths would answer this.
- Whether the parallelism the workflow uses is chosen on purpose or just grew. If writers keep growing, a single writer process may be cleaner than tuning waits.
- Whether there is a test that runs concurrent writers on every change. Without one, a regression would only show up in production runs, and only sometimes.
- Whether the logs should record how long a worker waited for the lock before it succeeded. Even successful waits are a warning sign that contention is rising, before anything fails.
