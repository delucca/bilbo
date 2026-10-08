---
id: 01M1K7V6PS087ZJY7BGYCRNVSF
created: 2026-09-03T05:58-03:00
---

# state-store spec: connection setup and busy timeout

This note replaces the earlier note about "state store must open". The new value is `PRAGMA busy_timeout = 10000`, which every state-store connection now runs when it opens. The earlier setting was 5000. The rest of this note records how the state-store is expected to behave, so nobody has to rebuild that from the code again.

## Scope

This covers the state-store, the SQLite-backed piece of PatchPilot that remembers what the tool has done across runs. It holds upgrade attempts, pull request status, and which targeted tests were chosen and what they returned. It does not cover the GitHub Actions workflow files or the Docker image build, except where they touch how the database file is reached.

## Naming

The component used to be called `pilotdb`. It is called `state-store` now. Old branches, old issue comments, log lines from older runs and some operator scripts may still say `pilotdb`. Treat the two as the same thing. New code, docs, log messages and notes should use `state-store` only. If you find `pilotdb` in live code, rename it. If you find it in history, leave it alone.

## The busy timeout rule

Every connection the state-store opens must run `PRAGMA busy_timeout = 10000` right after it is created, before any query or other pragma that might take a lock. This is a per-connection setting in SQLite. It does not persist in the database file. A connection that skips it falls back to the library default, which is to fail at once on a locked database. So the rule applies to every code path that opens a connection, not only the main one.

## Why the value changed

The earlier setting of 5000 was too short once several PatchPilot jobs ran at the same time against one database file. A writer holding the lock through a slow commit could make other jobs give up and report a locked database, even though waiting a bit longer would have worked. Those failures looked like random upgrade failures, but they came from contention, not from the dependency being upgraded. Doubling the wait made them go away in practice without hiding real deadlocks.

## What the number means

The value is in milliseconds. When a connection hits a lock held by another connection, SQLite retries inside the call until the wait is used up, then returns a busy error to the caller. It is a ceiling on one wait, not a fixed delay. Most calls never wait at all. A call that does wait blocks the Node.js event loop only if the driver is synchronous, so keep that in mind when you read the driver notes below.

## Where to set it

Set it in one place: the function that opens connections. Callers should never open a database handle directly. If a new code path needs its own connection, such as a short-lived script or a test helper, it goes through the same opener so it gets `PRAGMA busy_timeout = 10000` for free. Do not copy the pragma into call sites. Duplicated values drift, and that is how the old number survived in places after earlier changes.

## Order of pragmas

The busy timeout comes first among the pragmas run on open. Other settings, such as the journal mode and foreign key enforcement, can themselves need a lock. If the timeout is not yet in place when they run, they can fail on a busy database for the same reason the old value caused trouble. Keep that order when adding anything new to the opener.

## Interaction with journal mode

The state-store relies on the write-ahead journal mode so that readers do not block the writer and the writer does not block readers. The busy timeout matters mainly for writer against writer contention, which write-ahead mode does not remove. Only one writer can commit at a time. So a longer timeout helps writers queue up politely, while readers should rarely see it at all.

## Transactions and lock upgrades

A transaction that starts as a read and later writes can hit a busy error that the timeout cannot fix, because SQLite may refuse the lock upgrade at once to avoid a deadlock. The timeout does not help there. Begin a transaction in write mode when you know it will write. This keeps the wait inside the timeout, where it is useful.

## Running in GitHub Actions

In CI, several jobs may point at the same database file when a workspace or cache is shared. That is the situation that exposed the old value. The timeout is not a substitute for keeping the file local to one runner. If a workflow shares the file through a cache or artifact, only one job should write to it at a time. The longer wait is a safety margin, not a design for many concurrent writers.

## Running in Docker

Inside the container the database file sits on a mounted volume or on the container filesystem. Locking depends on the filesystem underneath. Network filesystems can make advisory locks unreliable, and then no timeout value is enough. Keep the file on a local volume. Nothing about the container setup needs to change because of the new value; the pragma is applied by the application at connect time, not by the image.

## Failure behavior

When the wait runs out, the caller gets a busy or locked error from SQLite. The state-store should not swallow it or retry in a loop around the wait. It should surface the error with enough context to tell which operation was blocked, and let the job fail clearly. A failure caused by the store must be reported as a store failure, never as a failed upgrade, so platform engineers do not chase the wrong problem.

## Logging

When a connection opens, the state-store logs once at debug level that the busy timeout was applied, using the component name `state-store`. Do not log it on every query. If the log shows no such line for a connection, that connection was probably opened outside the shared opener, and that is a bug to fix.

## Testing the setting

A test should open a connection through the opener and read the pragma back, checking it equals the expected value. Another test should hold a write lock on one connection and try a write on a second, checking that the second waits instead of failing at once. Keep that test fast by releasing the first lock from a timer well inside the limit. Do not write a test that waits out the full ceiling; it only slows the suite.

## Rollout notes

The change takes effect as soon as a process starts with the new code, because the setting is per connection. There is no migration and no change to the database file. Long-running workers pick it up on their next restart. Mixed versions can run side by side with no harm, though the older ones will still give up sooner on a locked database.

## Things not to do

Do not raise the value again just to quiet a flaky test. A longer wait hides a real ordering problem or a stuck transaction. If failures return, look first for a transaction held open across slow work, such as a network call to GitHub or a test run, and shorten it. Do not put the timeout in an environment variable without a clear need either; one fixed value in one place is easier to reason about.

## Open questions

We have not decided whether the value should become configurable per deployment. We have also not checked whether very large repository fleets need a separate database per repository to avoid contention altogether. If either comes up, write a new note and link back here instead of editing the rule above in passing.
