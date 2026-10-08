---
id: 01M0X98JJZAZ68PVD91AT32F8T
created: 2026-08-25T17:19-03:00
---

# state-store spec

This note specifies how the state-store behaves and what the rest of PatchPilot may assume about it. The state-store is the SQLite-backed place where PatchPilot keeps what it knows between runs: which repositories it watches, which upgrade pull requests it opened, which verification runs belong to them, and what happened in each. It is written in a hurry from what the team has settled, so it is plain and a bit blunt. Where a detail is not settled, it says so instead of guessing.

The one hard rule first. The state-store must open every connection with `PRAGMA busy_timeout = 5000` so that writers wait instead of failing. This applies to every connection, with no exceptions: the long-lived service process, short CLI invocations, the jobs that run inside GitHub Actions, test fixtures, and one-off maintenance scripts. A connection that skips it will fail with a busy error the first time another writer holds the lock, and that failure looks like random flakiness in a pull request pipeline. The pragma is per connection in SQLite, not per database file, so setting it once somewhere and assuming it sticks is wrong. It has to be applied in the single place where connections are created, and nowhere else should open a database handle directly.

Related note: [[verify-runner-verification-must]] covers what the verification side must do with the results it reads from and writes to the state-store. Read it before changing anything that touches the verification tables.

## Purpose and scope

The state-store exists so that PatchPilot can be stopped and restarted at any point without losing track of work. Platform engineers run it against many repositories at once, so a restart in the middle of a batch of upgrade pull requests is normal, not an emergency. After a restart, the process reads the state-store and continues. It must not reopen a pull request it already opened, must not rerun verification that already finished with a recorded result, and must not forget that a pull request was closed by a human.

What belongs in the state-store: identity and configuration snapshots for each tracked repository, the list of candidate dependency upgrades found for each repository, the pull requests created for those upgrades and their last known status, the verification runs attached to each pull request with their outcome and a short reason, and a log of important transitions with timestamps. What does not belong: raw test output, build logs, cloned working trees, and anything large. Those live in files or in the artifacts of the CI system, and the state-store holds only a reference to them. If a row would need to hold a multi-megabyte blob, the design is wrong and the blob should be stored elsewhere with a pointer kept here.

The state-store is not a queue. People are tempted to use it as one because it is convenient, and a few small pieces of the code do lean on it that way, but the guarantees below are about durable records, not about delivery ordering. If a real queue is needed later, it gets its own component.

The state-store is also not a cache of GitHub. GitHub is the source of truth for the pull request itself: whether it is open, merged or closed, who commented, which checks passed. The state-store records what PatchPilot last observed and what PatchPilot decided. When the two disagree, the next sync pass fixes the state-store, never the other way around.

## Connection rules

Every connection goes through one factory function owned by the state-store. Callers ask the factory for a handle and never construct one themselves. The factory is responsible for the following, in this order.

First, it opens the database file in the location given by configuration. The path comes from the environment or a config file and is never hard-coded in application code. In Docker the location is a mounted volume so that the file survives container replacement; if the volume is missing, the factory must fail loudly at startup rather than quietly create a fresh empty database inside the container layer, because that would look like total amnesia after the next deploy.

Second, it applies `PRAGMA busy_timeout = 5000` before doing anything else on the handle. Order matters here: other setup statements can themselves hit a lock, and they should already be covered by the wait. The value is in milliseconds, so a writer will wait up to five seconds for the lock before giving up. That is long enough to ride out normal write bursts from parallel jobs and short enough that a truly stuck writer surfaces as an error quickly instead of hanging a workflow for minutes.

Third, it turns on foreign key enforcement. SQLite leaves it off by default and it is also per connection, so it is easy to lose. Without it, deleting a repository can leave orphan pull request rows behind.

Fourth, it selects write-ahead logging mode so that readers do not block the writer and the writer does not block readers. This is a property of the database file once set, but the factory sets it anyway on open and checks the result, because a database on an unsuitable filesystem can silently refuse it. If the mode does not take, the factory logs a clear warning that includes the file location and continues, since a working slower mode is better than a crash, except in the CI environment where it should fail so the problem is noticed.

Fifth, it sets a synchronous level appropriate for durability of committed state. The team chose the safer setting over the faster one: losing a recorded verification result after a crash costs more than the small write latency.

The factory returns a handle with a small typed wrapper. The wrapper exposes prepared statements, a transaction helper, and a close method. Nothing outside the state-store should see raw SQL strings except through the repository modules described later.

## Concurrency model

The expected shape of use is many readers and a modest number of concurrent writers. Concurrent writers come from several places: the main orchestration process updating pull request status, verification workers recording results as they finish, and a periodic sync job that reconciles with GitHub. In GitHub Actions, several jobs of the same workflow may also write at once when they share a database file, which happens when the file is restored from a cache or artifact and merged back, or when a self-hosted runner shares a volume.

SQLite allows one writer at a time. The wait configured by `PRAGMA busy_timeout = 5000` is what turns that limit into short queuing instead of immediate errors. It is not a license for long transactions. The rules for keeping write contention low are these.

Keep write transactions short. Do the slow work, such as network calls and test runs, before opening the transaction or after closing it. A transaction must never span a call to GitHub or a call to a child process. If a piece of code needs data from the network to decide what to write, it fetches first, then opens the transaction, writes, and commits.

Take the write lock up front when a transaction is going to write. A transaction that begins as a read and then tries to upgrade to a write can fail with a busy error even when a wait is configured, because SQLite may refuse the upgrade rather than wait when it would deadlock. The transaction helper therefore has a mode for writers that begins in immediate mode, and any code path that writes uses it. Read-only paths use the plain mode.

Do not hold a read cursor open while writing from the same handle. Iterate to completion, or collect the rows, then write. Holding a statement open across a write has caused confusing behavior before.

Retries on top of the timeout. Even with the wait, a busy error can still escape when a writer is held longer than the limit. The transaction helper retries a small number of times with a short randomized delay, then rethrows. The error that finally surfaces must keep the original message so that logs show it was a lock problem, not a logic problem. Callers outside the state-store do not implement their own retry loops; if they feel the need, that is a sign the transaction is too long.

Processes versus threads. The Node.js process is single-threaded for JavaScript, but the native SQLite binding may block the event loop during a long statement. For that reason heavy queries, such as full-history reports, run in a separate worker or a separate process with its own handle, and each such handle goes through the factory so it gets the same pragma setup.

## Data model

The schema is small on purpose. This section describes the concepts and how they relate, not the exact column lists, which live in the migration files and should be read there when needed.

Repository. One row per tracked repository, holding the owner and name as GitHub knows them, the default branch, the settings that control how upgrades are proposed there, and timestamps for when it was added and last scanned. A repository can be paused, which is a flag and not a deletion; paused repositories keep their history and are skipped by scanning.

Upgrade candidate. One row per dependency upgrade that PatchPilot has found for a repository: which package, from which version, to which version, in which manifest. A candidate has a state, such as found, proposed, superseded or dismissed. A newer version of the same package supersedes an older candidate that has not been merged. Superseding is recorded, not deleted, so the history explains why a pull request was closed.

Pull request. One row per pull request PatchPilot opened, linked to its candidate and its repository. It records the number as GitHub assigned it, the branch name, the last observed status, and the last time it was synced. If a human closes the pull request without merging, the status reflects that and the candidate moves to dismissed unless a newer candidate appears. The distinction matters: PatchPilot should not keep reopening a pull request a person deliberately closed.

Verification run. One row per attempt to verify a pull request by running targeted tests. It records which tests were selected and why in short form, the start and end times, the outcome, and a reference to where the full output lives. A pull request can have several runs, for example after a rebase. Only the latest run counts for the current status, but all of them are kept. The rules for how outcomes are interpreted are in the related note, [[verify-runner-verification-must]], and the state-store only stores what it is told; it does not decide whether a run is good.

Event log. An append-only table of notable transitions: candidate found, pull request opened, run started, run finished, pull request merged or closed. Each row has a timestamp and a short message, and refers to the other rows by key. Nothing updates or deletes rows in this table during normal operation. It is the first place to look when someone asks why PatchPilot did something.

Schema version. A single-row table, or the equivalent built-in user version, records which migration level the file is at. The factory does not run migrations on its own; the application start path does, explicitly, after opening a handle.

Time values are stored as UTC in a consistent text form, never as local time. Identifiers from GitHub are stored as given and never parsed into numbers unless they are documented numbers, because mixing types in one column causes subtle comparison bugs in SQLite, which is lenient about column types.

## Migrations and compatibility

Migrations are forward-only files applied in order. Each runs inside one transaction so that a failure leaves the file at the previous level. The runner takes the write lock before checking the current level, so that two processes starting at the same moment do not both decide they need to apply the same migration. The loser of that race waits (thanks to the busy wait), then sees the new level and skips.

A migration must not assume it is the only thing running. Long-lived workers might still hold the old schema in their prepared statements. The rule is to make changes additive first: add a column or table, deploy code that can use both shapes, and only later remove the old shape in a separate migration. Renames are done as add, copy, switch, drop over separate releases. This costs time but avoids a class of outages where a worker crashes mid-batch because a column vanished.

Data migrations that touch many rows run in batches with a commit between batches, so that they do not hold the write lock for long and starve the other writers. A data migration that cannot be batched is a design smell and should be discussed before being written.

Downgrades are not supported. If a release has to be rolled back after a migration, the supported path is to restore the database file from the backup taken just before the migration, and accept losing what was written after. The start path takes that backup automatically when it detects pending migrations, into the same volume, and keeps only a few of the most recent. It does not back up when nothing is pending.

Compatibility between PatchPilot versions. A newer binary opening an older file migrates it. An older binary opening a newer file must refuse to run, with a clear message that names both levels, and must not attempt to write. This guard is important in GitHub Actions where a workflow might pin an old action version while the long-running service has already moved on.

Tests for the state-store run against real SQLite files in a temporary directory, not mocks. In-memory databases are fine for pure unit tests of the repository modules, but anything about locking, timeouts, or migrations must use a file, because in-memory databases do not behave the same under contention. Test fixtures also go through the factory, so that the busy timeout pragma is in effect and tests do not pass on a configuration production never has.

## Operations in Docker and CI

Docker. The container expects the database location on a mounted volume. The image does not ship a database file. On start, the entrypoint checks that the location is writable and that the volume is actually mounted rather than an ordinary directory in the container filesystem; if it cannot tell, it errs on the side of failing. Only one container should write to a given file at a time in normal operation. Running two replicas against the same volume works with SQLite locking on a local disk, but it is not supported on network filesystems, where file locking is unreliable and corruption is possible. If someone proposes to put the file on a shared network mount, the answer is no; use a single writer service and let other things talk to it.

GitHub Actions. Jobs that need state restore the file at the start and, if they wrote anything, publish it back at the end. Because several jobs may do this around the same time, a last-writer-wins restore can lose updates. The current approach is to keep the Actions jobs mostly read-only against the state-store and have them report results through the orchestrator, which owns the writes. Where a job really does need to write locally, it writes to its own scratch database file and the orchestrator imports the rows afterwards. Do not make jobs race to overwrite one shared cached file.

Backups. Besides the migration backups, a periodic backup uses SQLite's own online backup facility, not a plain file copy, because copying a file in write-ahead logging mode while writes happen can capture an inconsistent pair of files. Backups are written next to the database volume and also shipped out of the host by whatever the platform team uses for that; PatchPilot only produces the backup file and does not manage off-host storage.

Housekeeping. Old event log rows and old verification runs for pull requests that were merged or closed long ago can be pruned by a maintenance command. Pruning runs in small batches and takes the write lock briefly each time. Pruning never touches rows for pull requests that are still open. After a large prune, a vacuum may reclaim space, but it needs exclusive access and a lot of free disk, so it is a manual step done in a quiet window, not something the service does by itself.

Observability. The state-store logs when a statement waits for the lock longer than a small threshold, including how long it waited and which repository module issued it. These lines are the early warning that contention is growing. A burst of such lines does not mean the timeout is too low; it usually means some transaction is too long. Raising the timeout hides the cause. If the final busy errors do start showing up in logs, look for a transaction that spans slow work before touching the value.

Security. The database file may hold repository names and pull request metadata that customers consider private, so its file permissions are limited to the service user. No tokens or credentials are stored in it. If a token ever shows up in a row, that is a bug to fix and a secret to rotate, not a feature.

## Failure handling and open points

Crash safety. A crash mid-transaction leaves the file at the last committed state, which is the point of using transactions and durable sync. On restart, the application does a consistency pass: runs that are marked in progress but have no live worker are moved to an interrupted outcome, never straight to failed or passed, so that verification logic can decide to rerun them. This pass is itself a short write transaction through the normal helper.

Corruption. If the integrity check run at startup reports problems, the service refuses to start, preserves the bad file under a new name for later inspection, and tells the operator to restore from the most recent backup. It does not try to repair in place. Since GitHub is the source of truth for pull request status, a restore plus a sync pass recovers most of the lost information; what is lost permanently is PatchPilot's own decisions and history made after the backup, such as dismissals, so the operator should expect to re-dismiss some candidates.

Busy errors that still escape. If after the wait and the helper's retries a write still fails as busy, the caller gets a typed error that says so. Orchestration code treats it as retryable at a coarser level: the unit of work is put back to try again on the next cycle, and an event is logged. Verification workers that cannot record a result keep it in memory briefly and retry, but if the process is about to exit they write the result to a local spill file next to the output reference, and the orchestrator picks that up on the next pass. Losing a verification result silently is the worst outcome here, so the spill path exists even though it is rarely used.

Readers and stale data. Readers see a consistent snapshot per transaction. Code that reads status to decide on an action and then writes must re-check inside the write transaction that the status has not changed, or use a conditional update that only applies if the expected prior value is still there. This avoids two workers both acting on the same pull request. A conditional update that affects no rows is a normal outcome, not an error, and the caller backs off.

Open points, not yet decided. Whether to split the event log into its own file so that heavy appends cannot contend with status updates; the current guess is that it is not needed yet, but the question will come back if lock-wait logging shows the event log as the main source. Whether to offer a read-only replica for dashboards instead of letting them open the main file; for now dashboards open the file read-only through the factory, which still applies the busy timeout so their reads during checkpoints do not fail. How long to keep verification history by default; the platform teams have not given a requirement, so the pruning command has no default schedule and only runs when someone invokes it.

Things people get wrong, collected so far. Opening a handle directly with the SQLite binding for a quick script and forgetting the timeout, then blaming the service for lock errors. Starting a transaction as a reader and then writing. Doing a GitHub call inside a transaction. Pointing two services at one file on a network share. Testing locking behavior with an in-memory database. Assuming a restored cache file is current. Each of these has caused a real incident or a near miss, and the review checklist for any change touching the state-store asks about them explicitly.

If you change anything in this note, keep the busy timeout rule at the top intact. It is the one line of this spec that other components rely on without reading the rest.
