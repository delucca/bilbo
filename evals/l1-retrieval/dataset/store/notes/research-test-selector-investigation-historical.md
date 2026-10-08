---
id: 01K4Q4EN4GE46JDK7T8M4HXNH2
created: 2025-09-09T08:40-03:00
---

# test-selector skip rate across historical upgrade PRs

This note records what we found when we looked at how aggressively the test-selector skips tests on dependency upgrade pull requests in PatchPilot. The short version: in an investigation of 120 historical upgrade pull requests, the test-selector skipped at least half of the tests in 87 of them. That is a large share, and it is the reason this note exists. Skipping is the point of the component, but a skip rate this high means the quality of the selection matters a great deal, and we should not treat it as settled.

## Names and aliases

The component is called test-selector in the docs and in most code review talk. Its internal codename is `impactmap`, and you will still see that name in older branches, old issue titles and a few log prefixes. The short form `tsel` is also in use; `tsel` is short for test-selector. All three names point at the same thing. If you search the repository or the issue tracker for one name and find little, try the other two before concluding nothing was written about it.

In this note I use test-selector throughout, and only use the other names when talking about where they show up.

## What the test-selector does

PatchPilot opens pull requests that bump dependencies, then runs targeted tests to check each bump. The test-selector is the part that decides which tests count as targeted. It looks at what the upgrade touched, works out which parts of the consuming repository depend on the touched package, and picks the tests that cover those parts. Everything else is skipped to save time and CI minutes.

Platform engineers who maintain many repositories are the audience. They do not want a full suite run for every patch bump across dozens of repositories, so the selector has to be cheap and trustworthy at the same time.

## The investigation

We took a sample of 120 historical upgrade pull requests that PatchPilot had already handled. For each one we replayed the selection and compared the selected set with the full test set of the repository at that point in time. The goal was simple: measure how much the test-selector skips in practice, not how much we assumed it skips.

The sample was not hand-picked for being easy. It mixed small patch bumps, minor bumps and a few major ones, across repositories of different sizes and languages of test tooling. It is still a sample, so treat the figures as indicative and not as a guarantee for every repository.

## Headline result

In 87 of the 120 pull requests, the test-selector skipped at least half of the tests. So a clear majority of upgrades ran with well under half of the suite. The remaining pull requests ran more than half, and a few of those ran close to everything because the upgraded package sat near the root of the dependency graph.

The threshold we used was at least half skipped, not a precise percentage. We did not break the 87 down into finer bands in this pass. If someone needs the distribution, that is a follow-up, not something to infer from this note.

## Why the skip rate is high

Most upgrades touch packages that only a small part of a repository imports. When the import graph is narrow, the impact set is narrow, and the selector rightly leaves most tests alone. Patch releases of leaf libraries are the clearest case.

A second cause is that the selector leans on static import analysis. It sees direct and transitive imports, but it does not see behavior that flows through configuration, dynamic loading or runtime lookup. Those paths make the impact set look smaller than it really is. That is the part that worries me, because a high skip rate caused by blindness looks the same in the numbers as a high skip rate caused by a narrow change.

## Risk of over-skipping

A skipped test cannot fail. If the selector skips a test that would have caught a regression from the upgrade, the pull request looks green and the problem lands later. The investigation measured how much is skipped, not how much was skipped wrongly. Those are different questions, and we have not answered the second one.

So the 87 figure should not be read as a success number or a failure number. It is a description of behavior. Whether it is acceptable depends on how often the skipped tests would have mattered.

## What we did not measure

We did not measure false negatives, meaning tests that were skipped but would have failed. We did not measure flaky test interaction, where a skipped flaky test hides noise and makes the run look better than it is. We did not measure the time saved in a way that could be compared across repositories.

We also did not check whether the skip rate differs between repositories with strong test layering and repositories where most tests are end to end. I expect it does, but I have no data in this pass.

## Interaction with CI

The selected test list is handed to the GitHub Actions workflow that PatchPilot sets up for each pull request. The workflow runs only the selected tests inside a Docker container so the environment matches what the repository normally uses. Selection results are also stored in SQLite so later runs and reports can compare what was chosen over time.

Because results are stored, the replay approach used in the investigation can be repeated without new CI runs for the selection part. Checking the actual test outcomes would still need the full suite to be executed.

## Possible safeguards

None of these is decided. They are ideas that came out of looking at the numbers.

First, run the full suite on a schedule for a sample of upgrade pull requests and compare it with the selected run, to estimate false negatives directly. Second, force a wider selection when the upgraded package is imported from many places or is part of the build tooling. Third, add a floor so the selector never drops below a minimum share of the suite for major version bumps. Fourth, flag pull requests where the skip rate is very high so a reviewer sees it in the description.

## Open questions

How often would the skipped tests have failed? We need a false negative estimate before we can say the current behavior is safe.

Should the threshold for a widened selection depend on the kind of version bump, or on the size of the impact set? Both are plausible and the data to choose is not here yet.

Should the pull request description tell the reader how much was skipped? It would make the behavior visible, but it might also add noise to every upgrade pull request.

## How to reproduce the investigation

Pick a set of already merged upgrade pull requests, restore the repository state from just before each one, run the selector against the diff and record the selected and total test counts. Then compute the share skipped and count how many are at least half. Keep the sample selection method written down so another person can pick the same sample.

If the sample changes, the headline figure will change, so quote the sample size alongside the result. For this pass that was 120 pull requests, with 87 of them at or above the half-skipped mark.

## Where to look in the code

Search for the test-selector module first, then for `impactmap` and `tsel` to catch older references and log lines. The selection logic, the stored results and the workflow glue are separate pieces, and a change to the skip behavior will most likely touch the first of those only.

I did not read code for this note, so I am not pointing at specific files. Whoever picks this up should locate them from the names above.

## Next steps

Estimate false negatives with scheduled full runs on a sample. Break the 87 down into bands so we can see how many were skipped nearly everything. Decide whether a minimum selection floor is worth its cost. Revisit this note once the false negative number exists, because that number decides whether the 87 is fine or a problem.
