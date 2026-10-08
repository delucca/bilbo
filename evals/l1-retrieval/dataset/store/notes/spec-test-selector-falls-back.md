---
id: 01JR29AVMCH76AT13FX974WZS5
created: 2025-04-05T03:13-03:00
---

# test-selector spec

The test-selector decides which tests PatchPilot runs to verify a dependency upgrade pull request. It exists so that a bump of one library does not trigger every test in a repository, which is slow and noisy when a platform engineer maintains many repositories. This note records what the test-selector is supposed to do, the one hard rule about falling back to the full suite, and the reasoning behind the rest of the behavior. It is a spec, so where the code and this note disagree, treat it as a bug to look at, not as an automatic win for either side.

The main fact to remember: the test-selector falls back to the full test suite when a lockfile change touches more than 25 packages. The limit is set by the key `fullSuiteThreshold`, and its value is `25`. A change that touches exactly `25` packages does not trigger the fallback. A change that touches more than `25` does. Everything below is about how the test-selector gets to a narrow selection in the normal case and why the threshold is the escape hatch for the abnormal one.

## Purpose and scope

PatchPilot opens upgrade pull requests and then has to say whether each one is safe. Saying so requires running tests. Running everything on every pull request does not scale across many repositories, since the same CI capacity is shared and the upgrade pull requests arrive in bursts, often after a registry publishes a batch of releases or after a scheduled sweep. The test-selector narrows the run to the tests that could plausibly be affected by the dependency change in that pull request.

The test-selector is a decision component. It does not run tests itself. It takes a description of what changed and a description of the repository, and it returns a plan: either a list of tests or test groups to run, or an instruction to run the full suite. Execution belongs to the part of PatchPilot that drives GitHub Actions jobs and, where isolation is needed, Docker containers. The test-selector only produces the plan, and that plan is recorded so that a later reader can see why a given set of tests ran.

In scope for the test-selector:

- Reading the lockfile diff of an upgrade pull request and working out which packages changed.
- Mapping changed packages to the parts of the repository that depend on them.
- Mapping those parts of the repository to tests.
- Deciding to widen the selection, or to fall back to the full suite, when the mapping is unreliable or the change is too large.
- Writing down the reason for its decision.

Out of scope for the test-selector:

- Deciding whether to open the pull request at all. That is upstream of it.
- Deciding whether a failing test means the upgrade is bad. That is a verdict step after execution.
- Retrying flaky tests. Retry policy belongs to the runner.
- Merging. PatchPilot does not merge on the strength of a narrow selection alone; that policy is separate.

The guiding idea is that a narrow selection is an optimization with a cost: if it is wrong, an upgrade that breaks something is reported as safe. So the test-selector is built to prefer widening over narrowing whenever it is unsure. The fallback to the full suite is the widest possible answer and is always available.

## The full-suite fallback

The rule is simple. Count the distinct packages touched by the lockfile change in the pull request. If the count is greater than `fullSuiteThreshold`, which is `25` by default, the test-selector returns a full-suite plan and does not try to be clever.

A few details that are easy to get wrong:

- The comparison is strictly greater than. A change touching exactly `25` packages is still handled by the normal narrow selection. Only more than `25` triggers the fallback.
- The count is of packages touched, not of lines changed in the lockfile and not of top-level dependencies the pull request names. A single top-level bump can drag in many transitive packages, and those count. This is deliberate: transitive churn is exactly the kind of change where the dependency graph shifts under the code in ways that are hard to map to tests.
- A package counts once even if the lockfile shows it at several versions or in several places in the tree. The unit is the package name, not the entry.
- A package counts as touched if it was added, removed, or had its resolved version changed. A change that only reorders entries or rewrites metadata without altering a resolved version does not touch a package.
- The threshold is read from configuration through the key `fullSuiteThreshold`. If the key is missing, the default of `25` applies. A repository can set its own value. Lowering it makes the test-selector fall back to the full suite more often; raising it trusts the narrow mapping on bigger changes.

A minimal configuration fragment, using only the key and value discussed here:

```json
{
  "fullSuiteThreshold": 25
}
```

Why a threshold at all? The narrow mapping works by following the dependency graph from changed packages to the code that imports them, and from that code to tests. For a small change the graph walk is cheap and the answer is tight. For a very large change the answer converges on almost everything anyway, because so many packages changed that nearly every module is downstream of at least one of them. Past that point the narrow selection saves little and carries the risk of a missed edge in the mapping. Falling back to the full suite is then both safer and nearly as cheap. The number `25` was picked as a point where that tradeoff tips; it is a tuning value, not a derived constant, and it is expected to be adjusted per repository if the default turns out wrong.

The fallback does not depend on whether the mapping succeeded. The test-selector checks the count first. If the count is above the threshold it stops there. This ordering matters because it means a very large change never spends time on graph analysis that will be thrown away, and it means a bug in the mapping cannot leak into the large-change case.

When the fallback fires, the plan says so explicitly. The recorded reason names the threshold as the cause and includes the count of touched packages and the configured value of `fullSuiteThreshold`, so a reviewer looking at a pull request that ran everything can tell at once that it was the size of the lockfile change and not a failure of the selector.

## Normal selection

Below the threshold, the test-selector builds a narrow plan. The method is conservative by design and goes in stages. Each stage can widen the result but none may shrink what an earlier stage already added.

### Stage one: find the changed packages

The test-selector compares the lockfile before and after the upgrade and produces the set of touched packages as defined above. It also records for each one whether it is a direct dependency of the repository or only a transitive one, and whether it is a runtime or a development dependency. That distinction is kept because development-only packages, such as test helpers and build tooling, usually affect the tests and the build differently from runtime libraries.

If the lockfile cannot be parsed, or the before and after versions of it cannot both be read, the test-selector does not guess. It returns a full-suite plan and records that the cause was an unreadable lockfile. This is a separate path from the threshold, though the outcome is the same. Keeping the two causes distinct in the record is important, because an unreadable lockfile points at a tooling problem that someone should fix, while the threshold fallback is normal behavior.

### Stage two: find the code that uses them

For each touched package, the test-selector finds the source files that import it. It uses the repository's own import graph, built from the source tree, and then follows importers outward: a file that imports a touched package is affected, and so is any file that imports an affected file, up to the point where the chain reaches an entry point or a test file.

This transitive walk is what makes the selection safe for libraries that are wrapped by internal modules. If a team wraps an HTTP client in a thin internal module, a bump of the HTTP client reaches every caller of the wrapper, not only the wrapper's own tests.

Things that make this stage unreliable, and which cause the test-selector to widen:

- Dynamic imports whose target cannot be determined statically.
- Packages loaded by name from configuration, as happens with plugin systems.
- Packages that are consumed outside of source code, for example as command line tools invoked from scripts, or as build plugins.
- Packages whose types are consumed but whose runtime code is not, which still matter to a type check.

For these, the test-selector adds a broader group, such as every test in the same workspace package, instead of trying to trace the exact path. The rule is that anything the walk cannot see is treated as potentially affecting everything nearby.

### Stage three: map code to tests

Affected source files are mapped to tests in two ways, and the union is taken. First, tests that import an affected file, directly or through the same transitive walk, are selected. Second, tests that sit alongside an affected area by the repository's own layout conventions are selected, even if no import links them. The second rule is a safety net for tests that exercise code through a process boundary or a network call and therefore leave no import edge.

Tests that are marked as slow or as integration-level are handled with extra care. If any affected file is reachable from an integration test, the integration test is included. The test-selector does not skip integration tests to save time, because they are the ones most likely to catch a runtime incompatibility from a dependency bump.

### Stage four: add the always-run set

Every repository can declare a small set of tests that always run for any upgrade pull request, regardless of what changed. These are typically smoke tests that load the application, or checks that the build output is well formed. The test-selector adds them to every narrow plan. They are not counted against the narrow selection when the reason is recorded, and they do not change the threshold logic.

### Stage five: produce the plan

The result is a list of tests or test groups, plus the recorded reason. The plan is stored in the SQLite database PatchPilot uses for its own state, tied to the pull request, so that the verdict step and any human reviewer can read exactly what was chosen and why. The plan is also written in a form the GitHub Actions job can consume to decide what to run inside its container.

## Behavior around edge cases

This section lists situations where the right behavior was discussed and settled, so a later session does not have to work them out again.

**Exactly at the threshold.** A lockfile change touching exactly `25` packages is not above `fullSuiteThreshold` and is handled by the narrow path. This is the boundary case people most often misremember. The word in the rule is "more than".

**Threshold set to a very low value.** If a repository lowers `fullSuiteThreshold` to a small number, most upgrade pull requests in it will run the full suite. That is allowed and is a reasonable choice for a repository with a thin or untrusted test mapping. The test-selector does not second-guess it.

**Threshold set to a very high value.** If a repository raises `fullSuiteThreshold` far above the default of `25`, the test-selector will attempt narrow selection on very large changes. The result will often be close to the full suite anyway, and the risk of a missed edge grows. Nothing in the test-selector prevents this; a reviewer should treat it as a deliberate trust decision by the repository owner.

**Zero packages touched.** If the lockfile diff touches no packages, for example because only metadata changed, the test-selector returns the always-run set only and records that no packages changed. It does not fall back to the full suite, since the count is not above the threshold. If a pull request that claims to be an upgrade shows no touched packages, that is odd enough that the record should say so plainly so someone can look.

**Monorepos.** In a repository with several workspace packages and a shared lockfile, the count is taken over the whole lockfile change, not per workspace package. A single shared lockfile bump that touches many packages hits the threshold once for the entire pull request. After that check passes, the narrow selection is computed across all workspace packages that are affected, not only one.

**Several lockfiles.** If a repository carries more than one lockfile, the touched packages from all of them are unioned by name before counting against `fullSuiteThreshold`. This avoids a situation where each lockfile alone stays under the limit while the combined change is plainly large.

**Pull requests that also change source code.** Upgrade pull requests are normally lockfile and manifest changes only. If a pull request also modifies source files, for example because a codemod was applied to adapt to an API change, the files changed by hand count as affected directly, and their tests are added in the same way as tests for any other affected file. The threshold logic is unchanged: it counts only packages in the lockfile.

**Pre-release and yanked versions.** The test-selector does not treat these specially. A package whose resolved version changed is touched, whatever kind of version it moved to.

**Selection that would be empty.** The test-selector never returns an empty plan. If everything else yields nothing, the always-run set is returned. If a repository has no always-run set and the mapping yields nothing, the test-selector widens to the full suite and records that as the reason, on the theory that running nothing and calling the upgrade safe is the worst failure available.

**Mapping data is stale.** The import graph is rebuilt from the source tree at the commit under test, not read from a cache created earlier. A stale graph would silently drop new importers, so the cost of rebuilding is accepted.

## Recording and explaining decisions

Every plan carries a reason. The reason is not decoration; it is how people trust the test-selector. The record states which path was taken: threshold fallback, unreadable lockfile fallback, narrow selection, or widened narrow selection. For each path it includes the information needed to reproduce the decision from the inputs.

For the threshold fallback the record includes the number of packages touched and the configured value of `fullSuiteThreshold`. For narrow selection it includes the touched packages, the affected areas found by the walk, any place where the walk had to widen and why, and the final set of tests and groups. For the unreadable lockfile path it includes which side could not be read.

A reviewer reading a pull request should be able to answer three questions from the record alone: why did these tests run, why did the others not, and would a different threshold have changed the outcome. The third one is answered by having both the count and the configured value in the record. If the count is a little above `fullSuiteThreshold`, a reviewer can see that the fallback was marginal and decide whether the repository's setting is too low.

The record also marks whether the selection was widened by an unreliable mapping stage, so that repeated widening in one repository shows up as a pattern. A repository that is widened on nearly every pull request probably has a mapping problem, such as heavy use of dynamic loading, and is a candidate for either a better mapping or an explicit always-run set.

## Tuning and review guidance

The default of `25` for `fullSuiteThreshold` is a starting point. These are the signals that should prompt changing it for a repository:

- If narrow selections routinely miss failures that the full suite later catches, lower `fullSuiteThreshold` so that more pull requests get the full suite.
- If the full suite is firing often on pull requests whose narrow selection would have been fine, and the full suite is expensive, consider raising it, but check first whether the narrow plans on mid-sized changes already include most of the suite. If they do, raising the value gains little.
- If a repository has an unusually deep dependency tree, even a small top-level bump can touch many packages. The threshold will then fire on pull requests that look small. In that case the threshold is working as intended, because the count reflects real churn; changing it is a policy choice about risk, not a fix for a bug.

When changing the default itself, remember that it affects every repository that does not set its own value. A change to the default is a change in how much testing a large fleet gets, and capacity on shared CI will move with it. Treat that as a deliberate decision with a note, not as a casual edit.

## Testing the test-selector

The test-selector is checked mostly through small fixture repositories and fixture lockfile diffs, because its inputs are structured data and its output is a plan that can be compared directly.

Cases the checks should keep covering:

- A change below the threshold yields a narrow plan that includes the always-run set.
- A change with exactly `25` touched packages yields a narrow plan, not a fallback.
- A change with more than `25` touched packages yields a full-suite plan whose reason cites the threshold, the count, and the configured value of `fullSuiteThreshold`.
- A repository that overrides `fullSuiteThreshold` is honored in both directions, lower and higher.
- A transitive-only change counts its transitive packages toward the threshold.
- A package present at several versions counts once.
- An unreadable lockfile yields a full-suite plan with the distinct unreadable-lockfile reason.
- A mapping stage that cannot trace an import widens the result rather than dropping it.
- A change that would produce an empty selection never yields an empty plan.

Because the decision is data in and data out, these checks should not need Docker or a real GitHub Actions run. Those are exercised separately by the execution layer's own checks. Keeping the test-selector checks free of that machinery is what keeps them quick and means they can run on every change to the selector itself.

When a missed failure is reported in the field, the first step is to read the recorded plan for the pull request. The usual causes are, in rough order of frequency: a dynamic load the walk could not see, a test that exercises code across a process boundary with no import edge, and a threshold set too high for the repository. Add a fixture that reproduces the cause before changing the selector, so the fix stays fixed.

## Open questions

- Whether the count against `fullSuiteThreshold` should weight direct dependencies differently from transitive ones. Today every touched package counts the same. A weighted count might better reflect risk, but it would make the rule harder to explain in one sentence, which is a real cost.
- Whether development-only packages should count toward the threshold at all. They are included now because a build tool upgrade can break test setup as badly as a runtime library can break behavior.
- Whether the threshold should adapt to the size of the repository's suite, so that a tiny suite always runs in full and a huge one gets a tighter limit. Nobody has asked for this yet. A fixed value is easier to reason about and to record.

Until one of these is decided, keep the rule as written: more than the configured `fullSuiteThreshold`, default `25`, touched packages means the full suite.
