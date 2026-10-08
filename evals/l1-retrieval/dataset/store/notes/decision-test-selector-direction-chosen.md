---
id: 01JSHNF7SGZRV5XEV3EVTFKA0Q
created: 2025-04-23T12:50-03:00
---

# test-selector: general direction chosen

This note records the general direction the team settled on for test-selector, the part of PatchPilot that decides which tests to run when a dependency upgrade pull request is opened. It leaves out exact values on purpose. Thresholds, limits and timings change often and live in configuration, not here. What stays stable is the shape of the approach and the reasons for it, and that is what a later session needs so it does not reopen the argument.

The short version: test-selector picks tests by following evidence of what the upgraded package touches, and it widens its choice when that evidence is thin. It never narrows silently. When it cannot justify a small selection, it runs more, and it says why. Speed matters, but a missed regression in a repository that someone else maintains costs far more than extra minutes of CI.

## Why a selector exists at all

Platform engineers using PatchPilot look after many repositories. Most upgrade pull requests are routine: a patch release of a utility library, a transitive bump, a type definitions update. Running the full suite for each of them wastes runner time, clogs GitHub Actions queues, and trains people to ignore red builds because they fail for unrelated reasons. A targeted run gives faster feedback and keeps the signal cleaner.

But targeted runs are only worth having if people trust them. A selector that sometimes skips the one test that would have caught a break teaches reviewers to rerun everything by hand, and then the component has no value. So the whole design leans on trust first and speed second. Every choice below follows from that ordering.

We also decided the selector is an advisor to the verification step, not a gatekeeper of merges. It produces a plan; the runner executes the plan; the pull request reports what was run and what was left out. Humans keep the final call.

## The general approach we chose

The selector works from the dependency graph outward. Given an upgraded package, it finds the code in the target repository that imports or otherwise reaches that package, then finds the tests that exercise that code, directly or through a chain of imports. Tests that never reach the upgraded package through any such chain are candidates for exclusion.

We chose static reachability as the main signal because it is explainable. Anyone reading the pull request can see that a test was picked because it imports a module that imports the package. We rejected making it the only signal, because static analysis in a TypeScript and Node.js codebase misses dynamic requires, reflection-style loading, plugin systems, and configuration-driven wiring.

So the approach is layered. Static reachability gives the base set. Additional signals add to it, never subtract from it. Fallback rules widen it. The result is a plan that errs toward inclusion.

## Signals the selector combines

The base signal is the import graph described above. On top of that, the selector looks at which files in the target repository changed in the pull request, since an upgrade often rewrites lockfiles and sometimes touches code for compatibility. Tests near changed code are added.

A second signal is history. PatchPilot stores past runs in SQLite, including which tests failed on which kinds of upgrades. If a test has repeatedly failed after upgrades of a given package or a given category of package, it gets included even when static reachability says it is unrelated. History is treated as a hint that something is wired in a way the graph cannot see.

A third signal is test metadata that repository owners can provide: tags, ownership, and explicit declarations that a suite depends on a package. Owner declarations win over inference when they ask for more tests. They cannot be used to remove tests the graph says are relevant, unless the owner has marked a test as known to be irrelevant for a reason they wrote down.

## Prefer widening over narrowing

The central rule is asymmetry. When signals disagree, the larger set wins. When one signal is missing or stale, the selector behaves as if it would have added tests. A selection is only shrunk by positive evidence of irrelevance, never by the absence of evidence of relevance.

This matters in practice for a few cases. A repository with no usable graph data, perhaps because the build is unusual or the analysis failed, gets a broad run. A package that is a build tool, a compiler plugin, a test framework or a runtime shim gets a broad run, because its effects are not visible in imports. A major-level change in a package gets a broader plan than a patch-level one, though we do not encode the exact boundaries here.

We accept that this will sometimes run more than strictly needed. The team considered that an acceptable price and would rather tighten rules later from observed data than loosen them early from optimism.

## Explainability of every selection

Each plan the selector produces carries its reasons. For every included test or group, the plan records which signal pulled it in. For every exclusion, it records what justified leaving it out. This goes into the pull request description in a short, readable form, and the full detail goes into the stored run record.

We chose this because the most common reaction to a targeted run is suspicion, and the best answer to suspicion is a visible chain of reasoning. It also helps debugging: when a regression slips through, the first question is why the selector left a test out, and the stored reasons answer it without rerunning the analysis.

Reasons are kept in plain language with stable categories, so that dashboards and later analysis can group them. Free-form text is allowed as an addition but not as a replacement for the category.

## Fallback behavior

The selector has defined fallbacks, and they are part of the design, not an afterthought. If analysis of the target repository fails, times out or produces output the selector does not trust, the plan falls back to a broad run and says so. If stored history is unavailable or looks inconsistent, the selector proceeds without it and widens to compensate. If the set of tests it would choose turns out to be empty, that is treated as suspicious, not as success, and the plan widens.

A failure inside test-selector itself must never block a pull request from being opened or verified. The worst outcome of a selector bug is a slower verification, not a missing one. That rule shaped error handling throughout: errors are caught at the boundary, logged with context, and converted into a conservative plan.

We also decided that a fallback must be loud. It shows up in the pull request text and in run records, so a pattern of fallbacks for one repository gets noticed and fixed rather than quietly costing time forever.

## Where it runs and how it is packaged

The selector runs inside the same environment that verifies the upgrade. In most setups that means a GitHub Actions job, and for repositories that need an isolated environment it means a Docker container prepared for the target. We chose to run analysis next to the code, not through a remote service, so that it sees exactly the dependency tree the tests will see, including the freshly upgraded one.

The component is written in TypeScript on Node.js, like the rest of PatchPilot, and is kept as a library with a narrow interface: it takes a description of the upgrade and a view of the repository, and returns a plan. It does not run tests itself and does not talk to GitHub directly. That separation keeps it testable without network access and lets the runner and the reporting code evolve separately.

We considered running the analysis once per repository and caching it broadly. We chose a lighter approach: cache what is cheap to validate, rebuild what is not, and prefer a fresh analysis over a possibly stale one whenever the repository state is in doubt.

## Storage and history

SQLite holds the memory the selector draws on: past plans, past outcomes, and per-test failure patterns relative to upgrade categories. We chose SQLite because PatchPilot already uses it, it needs no extra service, and the data is modest and local to the deployment.

The selector treats this store as advisory. It reads from it, and the verification pipeline writes to it after runs complete. We deliberately kept the selector from writing during plan creation, so a crashed or abandoned plan cannot pollute history with outcomes that never happened.

History ages. Old failures count for less than recent ones, and records for tests that no longer exist are ignored. The exact decay and retention rules are tuning details and belong in configuration and code comments, not in this note. What we decided is the principle: history can add tests readily and can only reduce confidence in an exclusion, never create one.

## Handling flaky and slow tests

Flaky tests distort everything the selector learns. A test that fails at random will look correlated with many upgrades. We decided the pipeline should identify and mark flaky tests separately, and that the selector should not use flaky failures as strong evidence in either direction. A flaky test can still be selected for relevance reasons; it just does not earn extra inclusion from its noisy history.

For slow tests, we chose not to exclude them for being slow. Cost may influence ordering, so cheap and highly relevant tests run early and give early feedback, but it does not decide membership. Dropping a slow test because of its cost would reintroduce the exact silent narrowing we are avoiding.

If teams want to cap total verification time, that is a policy applied by the runner on top of a plan, visible as such, not something hidden inside selection.

## Repository owner control

Owners know their code. The selector offers a way for them to state expectations: always run a certain suite for upgrades of certain kinds of packages, treat a certain package as high risk, or mark a suite as not relevant to a given dependency with a written reason. We chose declarative configuration kept in the target repository, not settings inside PatchPilot, so the knowledge travels with the code and gets reviewed like code.

The rule for conflicts is the same asymmetry as before. Declarations that add tests are honored directly. Declarations that remove tests are honored only when they are explicit and justified, and the removal is reported in the pull request so reviewers can see it.

We also chose sensible defaults so that a repository with no configuration at all still gets a sound, conservative plan. Configuration refines behavior; it should never be required to get safe behavior.

## Alternatives we considered and set aside

Running everything every time was the simple baseline. It is safe, but it scales badly with the number of repositories and the size of suites, and it hides real signal in noise. We kept it as the fallback rather than the default.

A purely history-driven selector, choosing tests by statistical correlation with past failures, was attractive for its low setup cost. We set it aside as the primary method because it is weak on new packages, new tests and changed code, and because its reasons are hard to explain to a reviewer. It lives on as a supporting signal.

A dynamic approach, instrumenting test runs to record exactly which code each test executes, would give precise coverage-based mapping. We did not reject it permanently. It costs extra runtime and complicates the environment, and results go stale as code changes. We kept the plan interface open so that coverage-derived data can be added later as another signal that adds tests and, with enough confidence, supports exclusions.

A model-based approach, using a learned ranking of tests, was considered and deferred. It would add an opaque component to a system whose value rests on trust. If it returns, it returns as an advisor that can add tests, not as an authority that removes them.

## Testing the selector itself

The selector is verified the way it asks others to verify upgrades: against realistic situations and with a bias toward catching misses. We maintain fixture repositories that represent typical shapes, such as a plain library, a monorepo, a service with dynamic loading, and a project with unusual build steps. For each, we check that the plan contains the tests that must be there for known upgrade scenarios.

The key property tests are about inclusion: for scenarios where a break is known, the plan must include a test that would reveal it. Tests about exclusion are secondary, since a plan that is too large is a cost problem while one that is too small is a correctness problem.

We also replay past real upgrades against the selector when its logic changes, comparing the new plan against what actually failed. Any case where a once-caught failure would no longer be selected blocks the change until someone understands why.

## Measuring whether it works

We decided on two families of measures and agreed to watch them together. One is safety: how often a regression reached a repository that a full run would have caught. The team treats any such event as an incident worth a short write-up, and the selector rules are adjusted in response. The other is efficiency: how much verification work is saved relative to a full run, and how that translates into faster feedback.

Safety has priority in any disagreement. A gain in efficiency that comes with even occasional misses is not accepted. Targets for these measures are set by the team and revisited periodically; this note intentionally does not pin them.

We also watch how often fallbacks trigger, how often owner declarations override inference, and how often reviewers rerun the full suite by hand after a targeted run. The last one is a trust indicator: if people keep rerunning everything, the selector is not earning its place, regardless of what the other measures say.

## Rollout approach

The selector was introduced conservatively. At first it ran in a mode where it produced a plan and the pipeline ran the full suite anyway, comparing the two outcomes. That gave real data on misses before anyone depended on it. Repositories then moved to targeted runs gradually, starting with those that have solid suites and low risk, and keeping the comparison mode available for any repository where confidence is low.

We agreed that moving a repository to targeted runs is reversible at any time, and that regressions or repeated fallbacks are a reason to move it back while the cause is investigated. The decision to trust the selector is per repository, not global.

## Open questions

Some things are deliberately left open. How to handle monorepos where one package's tests depend on another's through build outputs rather than imports is still being refined. How far to trust owner declarations of irrelevance over time, as code drifts from the reason they wrote down, needs a staleness check. And whether coverage-derived mapping justifies its cost for most repositories is something the data will answer.

For now the direction stands: reachability as the base, additional signals that only add, conservative fallbacks, visible reasoning, owner control that favors more testing, and a rollout that earns trust repository by repository. Anyone changing test-selector should check a proposed change against that list, and if it makes the selector narrower in any situation, it needs evidence, not just a faster run.

## Guidance for whoever touches this next

Before changing selection logic, read the stored reasons for a few recent plans so you know what the output looks like to a reviewer. Keep the interface narrow. Keep errors converted into conservative plans. Do not let any new signal remove tests on weak evidence, and do not let a slow or noisy test fall out of a plan only because it is slow or noisy.

If you find yourself adding a special case for one package, ask whether it is really a category, and whether the right place is owner configuration or the shared rules. Special cases pile up quickly, and each one is a place where a silent miss can hide.

If this direction needs to change, write the new decision as an update to this note, with the reason and the evidence, so the history of why test-selector behaves as it does stays in one place.
