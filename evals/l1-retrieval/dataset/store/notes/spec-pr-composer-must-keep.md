---
id: 01KNZD54TPDDA9AQ8665Z80T3X
created: 2026-04-11T20:13-03:00
---

# pr-composer spec

This note describes what the pr-composer is supposed to do in PatchPilot, where its edges are, and the rules it has to keep. It is the component that turns a planned dependency upgrade into a pull request on GitHub, fills in the description, attaches the test results, and decides whether a new pull request may be opened at all. Platform engineers who maintain many repositories rely on it, so most of the rules below exist to keep it quiet and predictable. If you change behavior here, update this note in the same sitting.

The one hard limit to remember: the pr-composer must keep at most `5` pull requests open per repository, and the limit is controlled by the configuration key `maxOpenPrs`. Everything about queueing, ordering and deferral in this note follows from that limit.

## Purpose and scope

The pr-composer sits after the planner and before the verifier. The planner decides which dependency should move and to which version. The pr-composer takes that decision, prepares a branch, commits the change, opens the pull request, and writes the text a human reviewer will read. The verifier then runs targeted tests against the branch and reports back, and the pr-composer posts that result on the pull request.

What it owns:

- Deciding whether a repository has room for another pull request.
- Building the branch name, the commit, and the pull request title and body.
- Grouping upgrades when the configuration says several belong together.
- Applying labels, assignees and reviewers from the repository settings.
- Keeping an accurate record in SQLite of every pull request it has opened, so that the open count is known without asking GitHub every time.
- Closing or superseding its own pull requests when a newer upgrade replaces them.

What it does not own:

- Choosing versions. That is the planner.
- Running tests. That is the verifier, which runs inside GitHub Actions or a Docker container depending on the repository.
- Merging. PatchPilot opens and updates pull requests; merging stays with humans or with the repository's own merge automation.
- Anything about secrets for the target repository. It uses the credentials the runner hands it and nothing else.

## The open pull request limit

The central rule is a cap. For any one repository, the number of pull requests opened by PatchPilot and still open must never exceed `5`. The cap is read from the configuration key `maxOpenPrs`. When the key is absent, the default applies, and the default is `5`. A repository may set a lower value. A repository may also set a higher value if the platform team has allowed that in the shared defaults, but the shared defaults can forbid raising it, and in that case the higher value is ignored and a warning is logged.

How the count is made:

- Only pull requests that PatchPilot itself opened count. A human's pull request against the same dependency does not count.
- A draft pull request counts as open.
- A pull request that is closed without merging stops counting immediately.
- A merged pull request stops counting.
- A pull request that the pr-composer has superseded and closed stops counting once the close is confirmed, not before.

The count is checked at the moment the pr-composer is about to create a branch, not only when the work is queued. Between queueing and creating, other pull requests may have been merged or opened, so the check is repeated right before the push. If the repository is at the limit at that moment, nothing is pushed and the upgrade goes back into the waiting queue.

Do not treat the cap as a soft target. A burst of upgrades, for example after a large ecosystem release, is exactly the case the cap is for. Reviewers on big repositories have complained about floods of automated pull requests, and the cap is the answer.

## Where the count comes from

The authoritative number is what GitHub says is open. The local SQLite record is a cache that makes the common case cheap. The rule is:

- On every run, the pr-composer reconciles its local record with GitHub for the repository before deciding on capacity. Pull requests that GitHub shows as closed or merged are marked finished locally.
- If GitHub cannot be reached, the pr-composer falls back on the local record but treats the repository as at the limit if the local record has not been confirmed recently. It is better to open nothing than to overshoot.
- If the local record says fewer are open than GitHub does, GitHub wins and the discrepancy is logged so someone can look at why the cache drifted.
- If the local record says more are open than GitHub does, GitHub also wins, and the stale rows are closed out.

The drift most often comes from pull requests closed by hand while PatchPilot was not running. That is normal and not an error.

Identification of own pull requests relies on a marker the pr-composer writes into the pull request body and on the branch naming convention. Both are checked, because either can be edited by a person. If only one matches, the pull request is counted anyway, since undercounting is the dangerous direction.

## Queueing and ordering when at the limit

When the repository has no free slot, upgrades wait. The waiting queue is per repository and is kept in SQLite alongside the record of opened pull requests. Ordering rules:

- Security-driven upgrades go first, then upgrades that fix a known break, then everything else.
- Within the same class, older planned upgrades go before newer ones, so that nothing starves.
- Patch-level moves are preferred over major moves when the class is the same, because they are cheaper to review and more likely to pass.
- Upgrades that touch the same dependency are collapsed. Only the newest target version is kept in the queue, and the older entry is dropped with a note in the log.

When a slot frees up, because a pull request was merged or closed, the pr-composer is woken by the next scheduled run or by a webhook-style event from the workflow, and it takes the head of the queue. It takes only as many entries as there are free slots. It never opens a pull request just because the queue is long.

A repository that is permanently at the limit is a signal, not just a state. The pr-composer records how long the oldest queued upgrade has been waiting, and the reporting side surfaces that figure so the platform team can see which repositories are not keeping up with reviews.

## Branches and commits

Each pull request lives on its own branch. The branch name carries a fixed prefix that identifies PatchPilot, then the ecosystem, the dependency name in a safe form, and the target version. Characters that are not valid in a branch name are replaced. The name has to be stable for a given upgrade, so that a rerun updates the same branch rather than making a second one. This matters for the cap: a duplicate branch would quietly eat a slot.

Commits are small and honest:

- One commit for the dependency change itself: manifest edit and lockfile update, nothing else.
- A second commit only when generated files must change as a consequence, and it is labeled as generated.
- The commit message states the dependency, the old version, the new version, and the reason class (security, fix, routine).
- The author identity is the PatchPilot bot identity configured for the installation, never a person's.

The pr-composer rebases its own branches onto the default branch when they fall behind and the change still applies cleanly. It does not resolve conflicts by guessing. If the rebase fails, the pull request is marked as needing attention, and a comment explains why. A branch that cannot be rebased still counts against the cap until it is closed or superseded.

## Pull request text

The title is short and follows one pattern: the action, the dependency, the version change. Reviewers scan titles in a list, so the pattern is kept strict and predictable.

The body has fixed sections in a fixed order so that people learn where to look:

- What changed: dependency, old and new versions, and whether this is a patch, minor or major move.
- Why now: the reason class, and for security upgrades a pointer to the advisory.
- What was tested: filled in after the verifier reports, and shown as pending until then.
- Risk notes: breaking changes called out in the dependency's release notes, when the planner found any.
- Housekeeping: the marker used to identify the pull request as PatchPilot's, and a line explaining how to stop further updates for this dependency.

Release notes are summarized, not pasted. If the release notes are very long, the body links to them instead of embedding them. If they cannot be found, the body says so plainly rather than inventing a summary.

The body is regenerated on updates, but anything a human added below the marker section is preserved. The pr-composer never rewrites text it did not write.

## Grouping

Some repositories want related upgrades in one pull request: all packages from the same monorepo family, or all development-only tooling. The configuration can name groups. A group counts as a single pull request against the cap, which is the main reason groups exist: they let a repository with a tight limit still keep up.

Rules for groups:

- A group is opened with whatever members are ready. Members that become ready later join the existing open pull request if it is still open and not yet approved.
- If the group pull request has already been approved, late members are not added; they wait for a fresh group pull request, which needs a free slot like any other.
- A member that fails verification does not block the rest if the configuration allows partial groups. Otherwise the whole group is held back, and the pull request says which member caused it.
- Security upgrades are never grouped with routine ones, so that an urgent fix is not delayed by an unrelated failure.

## Interaction with the verifier

After the pull request exists, the verifier chooses which tests are relevant to the change and runs them. The pr-composer's part is small but strict:

- It passes the verifier the branch, the changed dependency, and the files touched.
- It waits for the result through the workflow status and a record the verifier writes, rather than polling in a loop.
- It posts the outcome as a comment and a status summary in the body: passed, failed, or could not run. Those are three different states and they are kept apart.
- A failed result does not close the pull request. It leaves it open, labeled, so a human can decide. It still counts against the cap.
- A could-not-run result, for example because the container image failed to build, is treated as an infrastructure problem and is reported separately from a test failure.

If a newer upgrade for the same dependency arrives while the earlier pull request is still open, the pr-composer prefers to update the existing branch and body over opening another pull request. Only when the existing pull request has been touched by a human in a way that would be lost does it close the old one and open a replacement, and then the old one is closed first so the cap is never exceeded even briefly.

## Failure handling

The pr-composer is expected to fail safely. The guiding rule is that a partial failure must never leave the repository with more open pull requests than allowed, and must never leave a pushed branch with no record.

Cases:

- Push succeeds but pull request creation fails: the branch is recorded as orphaned and retried on the next run. It is not counted as a pull request yet, but the next run checks for orphaned branches before opening anything new, so retries do not stack up.
- Pull request creation succeeds but the local write fails: the next reconciliation finds the pull request on GitHub through the marker and branch name and repairs the record.
- Rate limiting from GitHub: the run stops opening anything and records when it may continue. Pending work stays queued.
- Permission errors: reported once per repository per run with a clear message, and the repository is skipped, not retried in a tight loop.
- Concurrent runs for the same repository: a per-repository lock in SQLite prevents two runs from both seeing a free slot. If the lock cannot be taken, the second run exits without doing anything and says so in the log.

The concurrent-run case deserves care. The cap is only as good as the lock. Anything that opens pull requests outside the lock, including one-off scripts, can break the limit, and should be avoided.

## Configuration

Settings are layered: shared defaults for the whole installation, then per-organization values, then per-repository values. The most specific layer wins, except where a higher layer marks a key as locked.

Keys that matter for this component, described in words:

- `maxOpenPrs`: the cap on open PatchPilot pull requests per repository. Default is `5`. A lower value is always honored. A value of zero is allowed and means the repository is paused for new pull requests while existing ones continue to be updated.
- Group definitions, as described above.
- Labels, assignees and reviewers to apply.
- Whether draft pull requests should be used for upgrades that have not yet been verified.
- Whether security upgrades may exceed the usual ordering. They get priority in the queue but they do not get an exemption from the cap.

Invalid configuration is rejected at load time with a message that names the key and the layer it came from. An invalid value for `maxOpenPrs`, such as a negative number or text, makes the pr-composer fall back to the default for that repository and log a warning, rather than refusing to run, so a typo cannot silently halt upgrades everywhere or flood a repository.

## Observability

Operators need to answer a few questions quickly: why was no pull request opened for this repository, what is waiting, and what did the pr-composer decide on the last run. The logs and the SQLite records are arranged to make that possible.

- Every decision to skip opening is logged with the reason: at the limit, paused, locked by another run, rate limited, permission problem, or nothing planned.
- Each run logs the number of open pull requests it found, the limit it applied, and the number of free slots, so a reader can check the arithmetic.
- The queue exposes the age of its oldest entry and the number of entries per class.
- Counters are kept for pull requests opened, updated, superseded and closed, per repository, so trends are visible over time.

Logs avoid including credentials or full request bodies. Release note text is not logged beyond a short excerpt.

## Testing notes and open questions

Tests for the pr-composer lean on a fake GitHub layer and an in-memory SQLite database. The scenarios that must stay covered:

- Exactly at the limit: nothing opens, the upgrade stays queued.
- One slot frees up between queueing and creation: the check at creation time sees it and proceeds.
- A slot is taken by a human closing and reopening a PatchPilot pull request between check and push: the push is allowed only if the recheck still shows room.
- Local record stale in both directions.
- Two concurrent runs racing for the last slot: only one wins.
- Groups counted as one against the cap, including late members and approved groups.
- An invalid value for `maxOpenPrs` falls back to the default with a warning.

Open questions that have not been settled:

- Whether draft pull requests should count at a reduced weight. For now they count in full, since reviewers still see them in their lists.
- Whether a repository at the limit for a long time should have its oldest PatchPilot pull request closed automatically as stale. The current answer is no; humans decide.
- Whether the cap should be shareable across a set of closely related repositories. Not planned, but requested by one team.

Keep these in mind before changing the counting rules. Most bugs reported against this component so far were counting bugs, not text bugs.
