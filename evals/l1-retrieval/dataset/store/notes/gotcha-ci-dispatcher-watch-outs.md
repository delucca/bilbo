---
id: 01JSQHDENHGARF19JK7QV4BMDT
created: 2025-04-25T19:34-03:00
---

# ci-dispatcher gotchas

Notes on what tends to go wrong when touching ci-dispatcher. It sits between the part of PatchPilot that opens upgrade pull requests and the CI that runs the targeted tests, so a small change here can look fine locally and then misbehave across many repositories at once. Read this before editing, and add to it when something new bites.

## Assumptions about the CI side

ci-dispatcher talks to GitHub Actions, and most surprises come from treating that as a synchronous, reliable thing. It is neither.

- A dispatch request being accepted does not mean a run exists. The run shows up later, sometimes much later, and sometimes under a slightly different shape than expected. Code that dispatches and then immediately looks for "the run I just made" will pick the wrong one or find nothing.
- Matching a run back to a pull request by branch name alone is fragile. Branches get reused, force-pushed and rebased by the upgrade flow. Match on something that is unique to the dispatch, and treat a match on branch only as a guess.
- Workflow definitions live in the target repositories, not here. Repositories drift. Some will lack the expected workflow, some will have it renamed, some will have it disabled. Do not assume the input names a workflow accepts are the same everywhere.
- Rate limits and secondary limits apply per token and per installation. Many repositories processed in one pass can trip them even if each repository is only touched lightly. Any new call in a loop is a new place to hit a limit.
- Status and conclusion are different fields. A finished run can be cancelled, skipped, neutral or timed out, and none of those is the same as passing. Check how each is mapped before changing the mapping, and keep the default for unknown values on the cautious side (not verified).
- Webhook delivery, if used, can arrive out of order or twice. Polling, if used, can see stale data right after a change. Whichever path a change touches, make the handler safe to run twice.

## Local state in SQLite

The dispatcher keeps its own record of what it sent and what came back. That store is the thing that stops duplicate dispatches, so treat the schema and the write order with care.

- Write the intent before making the outbound call, and record the outcome after. If the process dies in between, the next start must be able to tell "sent but unknown" from "never sent". Reordering these two writes quietly breaks restart behaviour.
- Schema changes need a migration that works on an existing database file that already holds rows, not only on a fresh one. Test against a copy of a populated file. Adding a non-null column without a default is the usual way to fail this.
- SQLite locking bites when the dispatcher and some other worker share the file. Keep transactions short, do not hold one open across a network call, and do not assume a long read will see a consistent snapshot of rows another process is updating.
- Timestamps: be consistent about timezone and unit. Mixing stored values from different code paths has caused wrong ordering before, and ordering decides which run counts as the latest.
- Unique constraints are part of the dedupe logic. Loosening one to make an error go away usually just moves the problem to duplicate runs and duplicate comments on pull requests.
- Do not delete old rows to "clean up" without checking what still reads them. Retry and reporting code may look back further than you expect.

## Retries, idempotency and fan-out

Because one upgrade can touch many repositories, failure handling matters more than the happy path.

- Retrying a dispatch is only safe if the earlier attempt can be ruled out. Before adding or changing a retry, ask what happens if the first call actually succeeded and only the response was lost.
- Backoff should have jitter, and a cap. Without them a rate-limit episode turns into a synchronized burst when everyone retries together.
- One bad repository must not stall the rest. Keep failures scoped per repository and per pull request, and make sure an exception in one branch of the loop does not abort the whole pass.
- Concurrency limits exist to protect CI capacity and tokens. Raising them locally looks harmless; in production it competes with the teams' own pipelines. Check who else shares the runners before changing it.
- Cancelling superseded runs is useful but dangerous. If a newer upgrade replaces an older one, cancel only runs that this component started, and only for the same target. A broad cancel can kill someone's unrelated job.
- Targeted tests are chosen upstream. The dispatcher should pass the selection through untouched. If a change here starts filtering or expanding that list, the verification result stops meaning what the pull request says it means.

## Running it in Docker and in tests

- The container image has its own timezone, locale, and file layout. Behaviour that depends on the local clock or on a relative location of the database file can differ from a developer machine. Keep such things configurable and not baked into code.
- Credentials come from the environment. Never log request headers or full payloads at debug level; tokens end up in CI logs that other people can read. When adding logging, check what the object contains first.
- Tests that mock the GitHub API tend to encode the author's assumptions about timing. Add at least one case where the run appears late, one where it never appears, and one where the same event arrives twice.
- Fake timers in tests hide real polling problems. If you change an interval or a timeout, reason about it against real latency, not only against the fake clock.
- Do not point tests at real repositories. Use a throwaway repository or a recorded fixture, and keep fixtures small enough that a reviewer can read them.
- TypeScript types for API responses are written by hand in places. A field typed as always present may be missing in practice, especially on runs from older or odd workflows. Prefer a runtime check at the boundary over trusting the type.

## Before merging a change

Skim the list again and ask which of these your change touches. If it touches the write order, the matching of runs to pull requests, or the status mapping, say so in the pull request description so the reviewer looks there first. Prefer a small change that can be rolled back over a large one, since a bad release here fans out to every repository it manages before anyone notices.
