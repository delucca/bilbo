---
id: 01KRTCE290407W1BDJ0PVXRVXT
created: 2026-05-17T04:11-03:00
---

# pr-composer groups patch updates into one deps-patch pull request

pr-composer groups all patch updates into one pull request and labels it `deps-patch`. Before this, each patch bump became its own pull request, and reviewers were receiving about `40` separate pull requests per week. That volume was the reason for the change. Reviewers could not keep up, the pull requests were nearly all trivial, and the few that mattered got the same skim as the rest. This note records the decision, the reasoning, what we rejected, and what a later session should check before changing it.

```
pr-composer: all patch updates -> one pull request, label deps-patch
```

## Decision

When pr-composer builds pull requests from a batch of detected upgrades, every upgrade that is a patch-level version change goes into a single combined pull request. That pull request carries the label `deps-patch`. Upgrades of other kinds are not part of it. They keep their own handling and are not touched by this decision.

Short version for someone who only reads this paragraph: patch updates are batched, the batch is one pull request, the label is `deps-patch`, and the trigger for the change was reviewers getting about `40` separate pull requests per week.

The grouping is a property of pr-composer, not of the detection step that finds upgrades and not of the test runner that verifies them. Detection still reports each patch update individually. The test step still targets tests per changed dependency. pr-composer is the only place that decides how many pull requests come out of a set of upgrades. Keeping the decision there means the other components do not need to know that grouping exists.

The label name is part of the decision. `deps-patch` is how reviewers, filters and any automation downstream recognize the combined pull request. Renaming it is a behavior change for people, not a cosmetic one, so treat the name as stable.

## Why we did this

The problem was review load. PatchPilot is used by platform engineers who maintain many repositories. For each repository, every patch bump produced its own pull request. Across the repositories a reviewer was responsible for, that added up to about `40` separate pull requests per week. Most of them were small version bumps with no source changes on our side, only a lockfile or manifest edit.

The consequences we saw or expect from that volume:

- Reviewers stopped reading individual patch pull requests. They approved them in bulk or let them sit, which defeats the point of review.
- Notifications from the code host drowned out other work. People muted the bot, and a muted bot also hides the pull requests that do need attention, such as the ones with failing verification.
- Each pull request triggered its own CI run and its own targeted test run. That is a lot of repeated setup for changes that are individually low risk.
- Merging many small pull requests in sequence caused avoidable conflicts in lockfiles. Each merge invalidated the next one, and the bot had to rebase or regenerate.

A single pull request per batch turns that stream into one review action. The reviewer looks at one diff that lists every patch bump, sees one verification result, and merges once. The total amount of change is the same, but the number of decisions drops sharply.

Patch updates are the right thing to batch because they are the lowest-risk class. By convention a patch release is a bug fix with no intended interface change. Reviewers are mostly checking that nothing odd happened, not evaluating a design. That kind of check scales fine to a combined diff. Larger changes need a reviewer to think about one dependency at a time, so batching them would make review worse, not better.

## How it behaves

pr-composer takes the set of upgrades for a run and splits it by update kind. The patch group is merged into one pull request. The title and description list each dependency that was bumped, so the reviewer can see what is inside without opening the diff. The label `deps-patch` is applied when the pull request is created.

Things a reader should be able to answer from this note:

- What is grouped? All patch updates.
- How many pull requests does the group become? One.
- What label does it get? `deps-patch`.
- Why was it done? Reviewers were receiving about `40` separate pull requests per week.
- Which component owns it? pr-composer.

A few behaviors follow from the design and are worth stating plainly.

First, the combined pull request is only as good as its weakest member. If verification fails for one dependency in the group, the pull request shows a failure. That is the main cost of batching, and it is covered in the next section.

Second, the combined pull request has to be updated rather than duplicated when a later run finds more patch updates. The intent is that a reviewer sees a single open `deps-patch` pull request per repository at a time, not a growing pile. If a run finds new patch updates while one is already open, the open one should absorb them. Check the current code before relying on this, since the exact refresh mechanics are an implementation detail that may have moved.

Third, the label is how an open batch is found again. If someone removes the label by hand, pr-composer may not recognize the pull request as its own and could open a new one. Reviewers should be told not to strip it.

## Trade-offs we accepted

Batching has real downsides. We took them knowingly because the review-load problem was worse.

### One bad update blocks the group

If any single patch update breaks the build or a targeted test, the whole combined pull request is red. The reviewer has to find the culprit and either remove it from the batch or wait for a fix. With separate pull requests, the good bumps would have merged and only the bad one would have stalled.

The mitigation we expect is that a failing member gets split out of the group so the rest can proceed. Whether that is automatic or manual is something to confirm in the code. If it is not automatic yet, it is the first follow-up worth doing, because without it the batch can be held up by one flaky or incompatible package.

### Larger diff per pull request

The combined diff is longer than any single patch pull request. For patch updates that is acceptable because the diffs are mostly manifest and lockfile lines. It would not be acceptable for upgrades with code changes, which is another reason only patch updates are grouped.

### Harder to revert one piece

Reverting one dependency out of a merged batch is more work than reverting a single small pull request. Reviewers who care about easy per-dependency reverts can rely on the description listing each bump, but it is a manual revert of part of a commit. We decided this was rare enough to live with.

### Attribution in history

A merged batch commit mentions many dependencies at once, so searching history for a single dependency name relies on the commit or pull request body listing it. This is another reason the description must list every member and not summarize.

## Alternatives we rejected

### Keep one pull request per update and add throttling

Capping how many pull requests the bot opens per week would cut the volume without changing the shape. We rejected it because it delays updates arbitrarily and still leaves reviewers handling many tiny pull requests. A cap also hides which updates are being skipped, which is a worse failure than a large batch.

### Group everything, not just patch updates

A single pull request for all upgrades of every kind would be the biggest reduction in count. We rejected it because it mixes low-risk bumps with changes that need real attention. A reviewer would either rubber-stamp the whole thing or get stuck on the hardest item while the easy ones wait. Keeping patch updates separate means the easy path stays easy.

### Group by ecosystem or by directory

Splitting patch updates into several pull requests by package ecosystem or by area of the repository would keep each diff smaller and limit blast radius. It also brings back part of the volume problem, since a repository with several ecosystems would produce several pull requests again. We may revisit this if a single batch turns out to be too big in practice, but it was not the starting point.

### Auto-merge patch updates with no review

If verification passes, merging without a human would remove the review load entirely. We did not take this on. Platform engineers want a person in the loop for dependency changes, and auto-merge policy is a separate decision that belongs to the repository owners, not to the tool by default. Grouping is compatible with auto-merge later: a single labeled pull request is easier to target with a merge rule than many unlabeled ones, and `deps-patch` gives such a rule something to match.

### Let each team choose per repository

A per-repository switch for grouping was considered. We held off. A switch adds configuration surface and test cases, and the volume problem was general. If a team shows a strong need to opt out, adding a setting later is straightforward, and nothing in this decision blocks it.

## Things to check before changing this

A later session picking this up should confirm a few points in the code and not assume them from this note.

- Where in pr-composer the split by update kind happens, and whether patch is detected from the version change or taken from an upstream classification.
- How an already-open `deps-patch` pull request is found and refreshed on later runs. If it depends only on the label, note the fragility described above.
- What happens when a member of the group fails verification. Whether it is split out, left in, or blocks the whole pull request decides how painful the main trade-off is.
- How the pull request description is built, and whether it lists every dependency in the group. The reviewer-facing value of batching depends on that list being complete.
- Whether the label is created on the code host if it does not exist yet, or whether it must already be present. A missing label can make creation fail or leave the pull request unlabeled, which would break the lookup mentioned earlier.

If you change the grouping rule, update this note and keep the reason. The reason matters more than the mechanism: reviewers were receiving about `40` separate pull requests per week, and the grouping exists to fix that. If the volume problem goes away by some other route, for example by lower upgrade frequency, the case for batching gets weaker, and the trade-offs above should be weighed again.

## What would make us revisit

Signals that the decision should be reopened:

- Batches routinely stall because one member fails. That points toward automatic splitting or toward smaller groups.
- Reviewers report that the combined diff is too large to read. That points toward grouping by ecosystem or by area.
- Repository owners ask for auto-merge on verified patch updates. That is a policy layer on top of this decision and should reuse the `deps-patch` label.
- Teams ask to opt out. That calls for a setting, not a reversal.
- The label name collides with something already used by a team for other purposes. Then the name changes, and every filter and rule that matches it has to be told.

Until one of those shows up, the default stands: pr-composer puts all patch updates into one pull request labeled `deps-patch`, and the reason is the review load of about `40` separate pull requests per week.

## Notes for reviewers and operators

A short guide that can be pasted to the people who receive these pull requests.

The pull request labeled `deps-patch` is the combined patch update. It lists every dependency that moved. Read the list, check that verification is green, and merge. If verification is red, look at which dependency caused it, ask for it to be split out or fix it, and let the rest through.

Do not remove the `deps-patch` label. It is how pr-composer recognizes its own batch on later runs. Removing it can lead to a second pull request that overlaps with the first.

If you see a dependency update that is not in the batch, it is not a patch update. It follows the normal path, one pull request per upgrade, and should be reviewed with the care that implies.

If you think the batch is too large or too noisy, say so. That feedback is exactly what the revisit list above is waiting for, and the first thing to learn is whether the problem is size, failure handling, or labeling.

## Context for later sessions

This was settled as a decision, not as a design, so the details of the implementation are deliberately not recorded here. What is settled is the behavior and the reason. The code is the authority on how pr-composer does it today.

The decision touches only pr-composer. Detection of upgrades and the targeted test step were left alone on purpose. If a change seems to require edits in those components, stop and reconsider, because it probably means the grouping is leaking out of the place it belongs.

When writing tests for this behavior, the useful cases are: several patch updates produce exactly one pull request; the pull request has the `deps-patch` label; non-patch updates do not enter the group; and a second run with new patch updates refreshes the open batch instead of creating another. Those four cases cover the decision and the main way it could regress.

When writing user-facing text about this, keep the reason in plain terms. People accept a larger pull request more readily when they know it replaces a flood of small ones, and the figure of about `40` separate pull requests per week is the concrete thing that makes the case.
