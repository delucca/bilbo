---
id: 01JTDS9NZSEJSAKWSEDVB4H79C
created: 2025-05-04T10:55-03:00
---

# upgrade-scanner aborts on large runs with a rate limit error

The upgrade-scanner aborts with `HTTP 429: rate limit exceeded` when a single run covers more than `300` repositories. The run burns through the GitHub API quota partway through, the next call gets the 429, and the whole run stops. Nothing downstream (pull requests, targeted tests) happens for the repositories that were not reached. This note is about that trap and how to stay clear of it.

The internal codename of upgrade-scanner is `depscout`. You will see that name in older logs, branch names, config keys and chat history. Both names mean the same component. Search for both when digging through history.

## Symptom

A run that looked fine on a small set of repositories stops midway. The log ends with `HTTP 429: rate limit exceeded` and the process exits non-zero. Repositories earlier in the list may already have been scanned, later ones were never touched. It is easy to read this as a flaky network problem, but it is not one.

## Trigger

The trigger is scale in one run. Past `300` repositories in a single run, the number of API calls needed for dependency listing, manifest reads and version lookups exceeds what the GitHub API quota allows. Below that size the same configuration usually completes. The exact point depends on how many manifests each repository has and how much quota was already used by other jobs sharing the same token.

## Why it happens

The scanner makes several calls per repository and does not slow down as quota drops. It treats every repository as equally cheap. When the quota is exhausted, GitHub answers with the 429 and the scanner has no recovery path for it, so it aborts instead of waiting.

## Shared quota

The quota belongs to the token, not to the run. If other GitHub Actions workflows or other PatchPilot jobs use the same token, they eat the same budget. A run with fewer repositories can still hit the error if the quota was already low when it started.

## What not to do

Do not just retry the same run immediately. The quota has not refreshed, so it fails again at about the same place. Do not raise the repository list further hoping it will batch itself; it will not.

## Workaround

Split the work. Keep each run to `300` repositories or fewer, and space the runs out so the quota can recover between them. In GitHub Actions this means several scheduled jobs or a matrix with a limited slice each, not one big job.

## Choosing the slices

Group repositories in a stable way, for example by team or by owner, so the same repository always lands in the same slice. A stable split makes it easier to see which slice failed and to rerun only that one.

## Staying under the limit

Leave some margin below `300`. Repositories with many manifests cost more calls, so a slice of exactly that size can still fail. If a slice fails, make it smaller rather than rerunning it unchanged.

## Checking the quota

Before a large batch, look at the remaining quota on the token being used. If it is low, wait for the reset. This is a manual check for now, the scanner does not do it.

## Recovering after an abort

After an abort, work out which repositories were already scanned. The SQLite state the scanner keeps is the place to look for what completed. Then start a new run with only the remaining repositories, after the quota has reset.

## Local state

The SQLite database records scan results per repository. Do not delete it to get a clean rerun after a 429, since that throws away the record of what finished and makes the next run repeat the same expensive calls.

## Docker runs

When the scanner runs inside Docker, the container exits with the same failure. The container exit status is the quickest sign that the run aborted. The log line with the 429 is the cause.

## CI behavior

In GitHub Actions the job shows as failed, and later steps that open pull requests or run targeted tests are skipped. A red job with a 429 in the scanner step means the batch was too big or the quota was low, not that a dependency upgrade is broken.

## Longer-term fix

The real fix is for the scanner to read the rate limit headers, pause when the quota is nearly gone, and resume after the reset instead of aborting. It should also cap its own batch size. Until that exists, the manual limit on run size stands.

## Open questions

Nobody has measured exactly how many calls one repository costs on average. It would help to log call counts per run so the safe batch size can be set from data instead of guessed.

## Quick checklist

Check the run size against the limit of `300` repositories. Check the token's remaining quota. Split into stable slices. Keep the SQLite state. Rerun only what did not finish. If you see `depscout` in an old report with this error, it is the same problem.
