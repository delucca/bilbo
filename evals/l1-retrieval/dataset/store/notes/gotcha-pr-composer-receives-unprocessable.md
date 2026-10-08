---
id: 01KJDKJK50JSWF3PB2H1WBMW1W
created: 2026-02-26T15:32-03:00
---

# pr-composer gets 422 on push when the PR already exists

pr-composer fails with `422 Unprocessable Entity` from GitHub when it pushes a branch whose pull request already exists and was not closed first. It looks like a random API failure, but it is deterministic. Close the old pull request before pushing again, or the push is rejected every time.

## Symptom

A run of pr-composer for a dependency upgrade stops at the push or PR step. The log shows `422 Unprocessable Entity` coming back from GitHub. Nothing is wrong with the token, the branch contents or the network. Retrying without changing anything gives the same response, so a blind retry loop just wastes time and API quota.

## When it happens

The trigger is a re-run for an upgrade that pr-composer already opened a pull request for. Typical cases:

- A previous run opened the PR, then the targeted tests were re-run after a fix and pr-composer tried to publish again.
- A scheduled job picked up the same dependency bump a second time while the first PR was still open.
- Someone re-triggered the GitHub Actions workflow by hand for the same repository and the same branch.

In all of these the branch name is the same, and the PR for that branch is still open and was not closed first.

## Why

GitHub only allows one open pull request per head and base pair. When pr-composer pushes a branch and then asks for a PR on a pair that already has one, GitHub refuses and answers with `422 Unprocessable Entity`. The status is generic, so the message alone does not say "duplicate PR". You have to look at the response body and at the repository's open PR list to confirm.

## What to do

1. Check whether an open PR already exists for the branch pr-composer is about to use.
2. If it does and it is stale, close it first, then let pr-composer push again.
3. If the existing PR is still good, do not push a second time. Let the old one carry on and skip the run.

Closing first is the rule. Pushing over a branch with a live PR and hoping it updates cleanly is what produces the error.

## What not to do

- Do not treat `422 Unprocessable Entity` as transient and retry with backoff. It will not clear by itself.
- Do not delete the remote branch as a shortcut without closing the PR. That leaves a confusing closed or orphaned PR behind and loses the review history people may want.
- Do not swallow the error and mark the upgrade as done. The dependency bump was never published in that case.

## Guarding against it in pr-composer

The cleaner fix is a lookup before the push: ask GitHub for open PRs on the branch, and decide between close and reuse. This belongs in pr-composer itself, not in the workflow YAML, so every caller gets the same behaviour. Log a clear line saying that an existing PR was found, with which choice was made, so the next person reading the run does not have to decode a bare 422.

## Checking after a fix

Run the upgrade once on a repository that already has an open PR from pr-composer and confirm the old PR is closed and a new one is opened. Then run it a second time on a repository with no prior PR to make sure the normal path still works. Keep the SQLite run record in sync, so a closed PR is not still shown as open there.
