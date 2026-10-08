---
id: 01KT00YS0VRSXBS40331NW9QMB
created: 2026-05-31T19:02-03:00
---

# pr-composer open pull request limit spec

This note replaces the earlier note "pr composer must keep", which said the open pull request limit was 5. The new value is `maxOpenPrs` set to `3`. Anything that still reads the old figure from that note is out of date.

## What the limit does

pr-composer opens dependency upgrade pull requests on behalf of PatchPilot. `maxOpenPrs` is the cap on how many of those pull requests can be open at the same time for one repository. Once a repository has `3` open PatchPilot pull requests, pr-composer stops creating new ones for that repository until one of the open ones is merged or closed. Before this change the cap was 5, so a repository could carry two more bot pull requests at once than it can now.

The cap is per repository, not global across all repositories PatchPilot manages. Platform engineers who maintain many repositories will therefore still see plenty of bot activity overall, but each single repository stays quiet. If someone asks "how many open upgrade pull requests will pr-composer allow", the answer is `3`, set by `maxOpenPrs`.

## Why it went down

The old value was too noisy for the teams using this. Reviewers reported that a repository with several open upgrade pull requests at once got ignored: people stopped looking at the list, and stale bot pull requests piled up behind the fresh ones. Lowering the cap makes the queue short enough that someone actually reads it.

There is also a practical CI reason. Each pull request triggers a GitHub Actions run with the targeted tests. With fewer open pull requests there are fewer concurrent runs, less queueing on shared runners, and fewer cases where two upgrade branches touch the same lockfile and conflict with each other. Conflicting lockfile changes were a recurring annoyance, and a smaller cap reduces them without any extra logic.

## What counts as open

A pull request counts toward `maxOpenPrs` while it is open on the host, whether or not its checks have finished. A draft that pr-composer opened also counts. Merged and closed pull requests do not count. Pull requests that a human opened by hand do not count either, even if they are about a dependency; only the ones created by pr-composer are tallied.

How pr-composer recognizes its own pull requests matters here. It relies on the markers it adds when it creates a pull request, and on the records PatchPilot keeps in its SQLite database. If a marker is removed by hand on the host, that pull request may stop being counted. That is a known soft spot, not something this spec tries to fix.

## Order of selection when the cap is hit

When there are more pending upgrades than free slots, pr-composer has to choose. The rule is general rather than clever: security-relevant upgrades go first, then upgrades that have waited longest, then the rest in a stable order so repeated runs make the same choice. Upgrades that do not get a slot are not dropped. They stay pending and are picked up on a later run when a slot frees up.

Because the cap is now smaller, the pending backlog will be longer than before for busy repositories. That is expected. It is not a failure and should not raise an alert by itself.

## Interaction with grouping

pr-composer can group several small upgrades into one pull request. A grouped pull request occupies one slot, the same as a single upgrade pull request. Grouping is therefore the main way to get more upgrades through under the lower cap. The grouping rules themselves are unchanged by this spec. If the team wants more throughput, the preferred lever is to group more aggressively for low-risk packages, not to raise `maxOpenPrs` again.

## Configuration and rollout

`maxOpenPrs` is a setting that pr-composer reads when it starts a run. The shipped default is now `3`. A repository that sets its own value keeps that value; the default change only affects repositories that never set one. Engineers who had copied the old figure into their own settings should look at whether they still want it.

On rollout, repositories that already have more open PatchPilot pull requests than the new cap are not touched. pr-composer does not close anything to get under the limit. It simply opens nothing new until the count falls below `3`. That means a repository that was at the old maximum may take a while to start receiving new pull requests again, depending on how quickly people merge the existing ones.

The Docker image that runs PatchPilot picks up the new default with the next release; nothing in the image needs to change beyond that. The GitHub Actions workflows that call pr-composer do not pass the limit themselves, so they need no edit.

## Checks and open questions

When verifying this behaviour, look for these cases:

- A repository with fewer than `3` open pr-composer pull requests gets a new one on the next run.
- A repository with exactly `3` gets none, and the pending upgrades are still recorded as pending.
- Merging or closing one of the open pull requests lets exactly one more through on the following run.
- A repository with its own explicit `maxOpenPrs` ignores the default.
- A repository over the cap because of the old limit is left alone and simply waits.

Open questions that nobody has settled: whether security upgrades should be allowed to go over the cap in an emergency, and whether the cap should eventually be set per team rather than per repository. For now neither is supported, and the cap is a hard limit of `3` with no exceptions. If one of these gets decided, update this note instead of writing a new one next to it.
