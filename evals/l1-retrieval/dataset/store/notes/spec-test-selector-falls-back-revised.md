---
id: 01JXN6GN7V76WPX4FHRN34QK33
created: 2025-06-13T14:48-03:00
---

# test-selector full suite threshold

This note replaces the earlier note about "test selector falls back". The new value is that the test-selector now sets `fullSuiteThreshold` to `40`, which replaces the earlier value of 25. Read this note as the current spec for that setting. The older note should be treated as out of date wherever it quotes the old number.

The short version: when an upgrade pull request touches a small enough set of affected tests, test-selector runs only those tests. When the set grows past the threshold, test-selector stops trying to be clever and asks the workflow to run the whole suite for that repository. The threshold decides where that switch happens. It moved up, so more upgrade pull requests now get the targeted run and fewer get the full run.

```json
{
  "fullSuiteThreshold": 40
}
```

## What the setting means

`fullSuiteThreshold` is a count of selected tests, not a percentage and not a time budget. test-selector looks at the dependency change in the pull request, works out which test files could be reached through that dependency, and counts them. If the count is above `fullSuiteThreshold`, the selection is thrown away and the full suite is requested. If the count is at or below it, the selected tests are what runs.

The value is now `40`. Before, it was 25. So a pull request whose selection used to be judged too wide, and therefore fell back to the full suite, may now stay targeted. A pull request that selected a modest number of tests before behaves the same as it did, because it was already under the old limit and is still under the new one.

A few points that are easy to get wrong:

- The threshold applies per pull request, per repository. It is not summed across repositories, even though platform engineers run PatchPilot over many of them.
- The count is taken after test-selector has removed duplicates. Two paths that lead to the same test file count once.
- The fallback is a decision, not an error. When test-selector falls back to the full suite it should log that it did and why, and the pull request check should still pass or fail on the test results as usual.
- Nothing here changes how a selection is computed. Only the cut-off for accepting it changed.

## Why it was raised

The old limit was low enough that ordinary upgrades kept tripping it. A shared library bump, such as a utility or logging package, is imported from many places, so its reachable test set is wide even though the real risk is small. With the old value those pull requests went to the full suite almost every time, and the full suite is the slow path. On repositories with long test runs this made upgrade pull requests wait in the GitHub Actions queue and used runner time that other work needed.

Raising the limit to `40` is a trade. We accept that a targeted run on a wider selection might miss an interaction that only the full suite would catch. In exchange we get faster feedback on the large middle group of upgrades that touch a moderate number of tests. The judgment at the time was that the middle group is common enough, and low enough risk, that the trade is worth making. If that turns out wrong, the value is the first thing to look at, before touching the selection logic.

There is no claim that `40` is a tuned optimum. It was picked as a step up from the earlier value that clearly relieves the common case without making the selection so wide that it stops being targeted. Treat it as a default that can move again.

## How it fits with the rest of PatchPilot

test-selector sits after the step that opens the upgrade pull request and before the step that runs tests in GitHub Actions. The pull request itself is produced upstream; see [[pr-composer-creates-upgrade]] for how that part works. test-selector receives the dependency change from there and hands a test list, or a full suite request, to the workflow that runs inside Docker.

The selection history and the decisions it made are kept in SQLite, so a platform engineer can look back at why a given pull request got a targeted run or a full run. When the threshold changes, old rows keep the value that was in force when they were written. Do not rewrite them to the new number. If you compare behavior before and after this change, compare by when the row was recorded, not by the current setting.

The setting lives in the test-selector configuration and is read when the service starts, so a changed value takes effect for pull requests handled after the next start. Runs already in flight keep the value they began with. If you change it in a hurry and nothing seems different, check that the process was restarted and that the container picked up the new configuration rather than an older image.

## Checking and watching it

After any change to this value, confirm three things. First, that the running test-selector reports `40` for `fullSuiteThreshold` in its startup output or config dump. Second, that a pull request with a wide but not huge selection now gets a targeted run. Third, that a pull request with a truly huge selection still falls back to the full suite, so the safety net is intact.

Watch for these signs that the value is too high:

- Upgrade pull requests that pass the targeted run and then break the default branch after merge.
- Maintainers reporting that a failure only showed up in the full suite and would have been caught if the fallback had triggered.
- Selections that are routinely close to the limit, which suggests the number is doing more work than the selection logic should.

And for these signs that it is too low:

- Most upgrade pull requests still falling back to the full suite.
- Long queue times on the runners that line up with upgrade bursts.

If either set of signs shows up, adjust `fullSuiteThreshold` first and record the new value here, replacing the number above rather than stacking another one beside it. Keep this note as the single place that states the current value, so nobody has to guess which note is right.

## Open questions

It is not settled whether one value should serve every repository. Some repositories have small, fast suites where the full run is cheap, and a lower limit would cost little there. Others have huge suites where even a wide targeted run saves a lot. A per-repository override is a possible later change, but nothing in this spec adds one. For now there is one value, and it is `40`.

It is also not settled whether the count should weigh tests by how long they take. A raw count treats a quick unit test and a slow integration test the same. That was left alone on purpose, to keep the change small and easy to reason about.
