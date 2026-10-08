---
id: 01K6NP1C89VW7XME53HPR5VJYM
created: 2025-10-03T15:41-03:00
---

# state-store: things to watch out for when changing it

Notes for anyone about to touch state-store. These are general traps, not a spec. Read the section that matches what you are changing, then check the code, because details drift faster than this note does.

## Why state-store is touchier than it looks

state-store holds what PatchPilot knows between runs: which upgrade branches exist, which pull requests are open, what the targeted tests said last time, and what has already been tried and rejected. It looks like a small SQLite wrapper. It is really the memory of the whole tool. If it forgets something, PatchPilot opens duplicate pull requests. If it remembers something wrong, PatchPilot skips repositories that need work. Both failures are quiet. Nobody sees an error, they just see odd behavior a few days later.

Treat any change here as a change to every other component, even when the diff is five lines.

## Schema changes and existing data

Platform engineers run this against many repositories, and the database files are long-lived. Assume there are old files out there written by older builds. A schema change that works on a fresh database proves nothing.

- Always test the upgrade path from an old file, not only the empty case.
- Adding a column with a default is usually safe. Renaming, dropping or retyping is where SQLite makes you rebuild the table by hand.
- Do the rebuild inside one transaction. A half-migrated file is worse than an unmigrated one.
- Do not assume migrations run once. Concurrent starts can race, so a migration has to be safe to attempt twice.
- Keep the migration order stable. Editing an old migration after it shipped means some files have the old shape and some have the new one, with no way to tell which.

## Downgrades and mixed versions

Different CI jobs may run different builds of PatchPilot against the same state-store at the same time, especially during a rollout. A newer build that migrates the file can lock out an older build that is still running somewhere. Decide what the old build does when it sees a shape it does not know: fail loudly is better than guess. Do not add a migration that only makes sense if every runner updates together.

## Concurrency and locking

SQLite is a single-writer database. In GitHub Actions many jobs can start close together, and if they share a store through a cache or a mounted volume, they will collide.

- Long write transactions block everyone else. Keep them short, and never hold one open across a network call to GitHub or a test run.
- Do reads outside write transactions where you can.
- Busy handling needs a sensible wait, not an immediate failure and not an endless one. Check what the current setting is before changing anything near it.
- A read that turns into a write halfway through a transaction can fail in ways that only show up under load.
- Write-ahead journaling changes which files sit next to the database. Anything that copies, caches or uploads the store has to take all of them, or it will save a stale or broken copy.

## Where the file lives, and what happens to it in CI

In Docker and in GitHub Actions the working filesystem is often thrown away at the end of a job. Persistence only exists if something saves and restores the file on purpose. When you change how or where the store is opened, check the whole round trip: restore, open, write, close, save.

- Closing cleanly matters. A job killed mid-write can leave the file in a state that the next job restores happily and then chokes on.
- Cache restore can hand back an older copy than the last writer produced. Do not assume the file you opened is the latest one.
- Network filesystems and some container mounts do not give the locking SQLite expects. Do not move the store onto one without testing real concurrent use.
- File permissions inside the container can differ from the host. A store written as one user may be unreadable by the next step.

## Idempotency of writes

PatchPilot retries. Jobs get re-run by hand, by timeouts and by workflow re-triggers. Every write into state-store should be safe to repeat.

- Prefer upserts keyed on something stable over blind inserts.
- Be careful what you pick as the key. Anything that changes between runs, such as a timestamp or a generated name, will create duplicates.
- Recording that a pull request was opened should happen in a way that survives a crash between opening it and recording it. Think about which order is less harmful when the process dies in the middle: a duplicate pull request or a missed record.
- Do not let a retry overwrite newer data with older data.

## Time, ordering and clocks

Rows often carry times, and logic downstream decides things like whether a failure is recent or whether to back off. Runners do not share a clock exactly.

- Do not compare times written by different machines as if they were exact.
- Be explicit about time zone and format. Mixing local and universal times in one column causes bugs that appear twice a year.
- Do not rely on insertion order as a real order. If ordering matters, store something that says so.
- Sorting text that was meant to be a number gives wrong answers that look right on small data.

## Types, nulls and what SQLite lets through

SQLite is forgiving about types, and that hides mistakes. A value of the wrong type goes in without complaint and comes out later as something the TypeScript side did not expect.

- The type annotations in the TypeScript layer are not enforced by the database. Validate on the way in and on the way out.
- Booleans, large integers and dates need an agreed encoding. Changing the encoding is a schema change even if the table does not change.
- Null versus empty versus missing are three different things in the logic. Do not collapse them when refactoring.
- Foreign key enforcement is a per-connection setting. Check that every connection path turns it on, including tests and scripts.

## Tests that touch state-store

Targeted tests are the point of PatchPilot, so a broken test setup here is embarrassing and also misleading.

- Use a fresh store per test. Shared state between tests produces failures that depend on order.
- An in-memory store behaves differently from a file store for locking and for journaling. Run at least some tests against a real file.
- Keep a fixture of an old-shaped store and run migrations over it. Update it only when a migration is deliberately retired.
- Clean up temporary files, or the CI runner fills up and unrelated jobs fail.
- Do not mock the store so heavily that the test only checks the mock.

## Growth, cleanup and performance

The store grows as more repositories and more runs pile up. Nothing in a small test shows this.

- Check that queries use an index when they filter on a column that is not the key. Adding a query without thinking about this is the usual way it gets slow.
- Deleting old rows does not shrink the file by itself. If size matters for caching, plan for compaction, and remember that compaction needs free space and exclusive access.
- Retention rules belong in one place. If two components each prune on their own terms, they will eventually delete each other's data.
- Large payloads such as full test output are better kept elsewhere, with a reference stored here.

## Secrets and sensitive data

Do not put tokens, credentials or private repository contents in the store. It gets cached, copied between jobs and sometimes uploaded as an artifact. Anything written here should be treated as readable by anyone who can see the CI cache. Also watch log lines that print rows while debugging, since they end up in shared CI logs.

## Before you merge

A short list to run through:

- Did I test against an old database file, not just a new one?
- Can two jobs run at once without one failing or corrupting the other?
- Is every new write safe to repeat?
- Does the save and restore path in CI still carry everything it needs?
- If an older build opens this file, does it fail clearly?
- Did I add anything sensitive, or anything unbounded in size?

If the answer to any of these is not known, find out before merging. Fixing a damaged store after it has been copied into many repositories' caches is much harder than the check.
