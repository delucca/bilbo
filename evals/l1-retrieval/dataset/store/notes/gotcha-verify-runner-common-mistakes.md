---
id: 01KK1VC38MRXE2DA9DD988ZR5C
created: 2026-03-06T12:13-03:00
---

# verify-runner gotchas

Most of the trouble people have with verify-runner is not in the code. It is in what they assume the component is doing on their behalf. It looks like a thin wrapper that takes an upgrade pull request, picks some tests and reports green or red. In practice it makes a lot of small judgments, and each one can quietly go wrong. This note is a list of the mistakes I keep seeing, in rough order of how much time they waste. There are no exact errors here on purpose. The symptoms vary between repositories, and chasing a message you remember is how people end up fixing the wrong thing.

Read it before you change anything in verify-runner, and again before you tell a platform team that a green check means an upgrade is safe.

## Treating a green result as proof the upgrade is safe

This is the biggest one. verify-runner runs targeted tests. Targeted means a subset, chosen by a mapping from the changed dependency to the code that uses it. A green result means the chosen subset passed. It does not mean the whole suite would pass, and it does not mean the upgraded library behaves the same in places the mapping did not reach.

Common ways this bites:

- The mapping is built from import statements, so anything loaded indirectly is invisible. Dynamic requires, plugin systems, config-driven loaders and anything reached through a re-export in another package all hide usage from the selector. The upgrade touches code the runner never exercised, and the check passes.
- A dependency that changes behavior at build time, such as a compiler plugin, a bundler loader, a lint rule set or a type definition package, has almost no runtime import to trace. The selector may pick nothing, or pick a token test, and report success. Treat these upgrades as needing the full suite, every time, and make sure the repository's configuration says so rather than relying on a person remembering.
- Transitive upgrades. A pull request that bumps one direct dependency may move several indirect ones in the lockfile. The selector often keys on the direct name only. The indirect ones that moved are the ones nobody looked at.
- Peer dependency shifts. The direct package may stay put while a peer it expects changes underneath. The targeted tests for the direct package pass, and the thing that breaks is in a different package's tests.

When a reviewer approves purely because the check is green, the failure shows up later on the main branch, where the full suite finally runs. Then people blame the upgrade bot. The fix is cultural as much as technical: the check name and the pull request comment should say plainly that this was a subset run, and reviewers should know what the subset was.

Also watch for the opposite mistake. When the selector finds nothing to run, some people read that as "nothing affected, safe." An empty selection is a signal that the mapping failed or that the dependency is used in a way the mapping does not understand. It should be treated as unverified, not as passed. If verify-runner reports success on an empty selection, that is a bug in how the result is classified, and a reviewer should be able to tell the difference at a glance.

## Assuming the verify environment matches the real one

verify-runner runs the tests in a container. The container is meant to resemble the repository's normal test environment. It resembles it less often than people think.

Mistakes in this area:

- Assuming the base image is the same one the repository's own pipeline uses. Repositories drift. One team moves their runtime forward and nobody updates the image used for verification, or the reverse. Tests pass under an older runtime and fail under the one in production, or the other way around. When a verification result disagrees with the repository's own pipeline, compare the runtime and the operating system layer before you suspect the upgrade.
- Forgetting native modules. A dependency with a compiled component needs build tools and system libraries present in the image. If the image lacks them, installation fails or, worse, falls back to a prebuilt binary that does not match. The failure looks like an upgrade problem and is really an image problem.
- Caching layers that hold stale installs. If the install step is cached on something that does not include the lockfile change, the container tests the old tree. This is rare but ugly: the result is green because nothing was actually upgraded. Always confirm the verification actually installed the new dependency tree, not the cached one.
- Network access. Some tests reach out to services. In the container they may be blocked, mocked differently or slow. People then add retries or skip markers to make the check pass, which erodes what the check means. Fix the test or the environment, not the signal.
- Environment variables and secrets. The repository's pipeline may inject values that the verification container does not have. Tests that depend on them are either skipped silently or fail in confusing ways. Make a deliberate list of what the runner is allowed to pass in, and keep secrets out of it. Do not copy the full pipeline environment into the container to make tests green. That leaks credentials into a place that runs untrusted upgrade code.

Speaking of untrusted: an upgrade pull request brings in code from a third party, and verify-runner executes it. Install scripts and test code both run. Treat the container as hostile. Do not mount anything you would not hand to the package author. Do not give it write access to the host's repository checkout beyond what it needs. Do not reuse a long-lived container between runs, because one run can leave something behind for the next.

## Mishandling the SQLite state

verify-runner keeps its run history and selection data in SQLite. It is a local file, which makes it easy to forget that it is shared state with real concurrency concerns.

The recurring mistakes:

- Running more than one verify-runner process against the same database file without thinking about locking. SQLite handles this, but only up to a point. Long write transactions block others, and a process that holds a transaction open while it waits on a container will starve everything else. Keep transactions short. Never hold one open across a call that waits on a test run.
- Putting the database on a network filesystem or a shared volume that does not honor file locking properly. It appears to work until two jobs write at once, and then it corrupts or loses rows. The database should live on local disk of whichever worker owns it, and the results that matter should be reported out to something durable.
- Assuming the database persists between CI jobs. On hosted runners the workspace is thrown away after a job. If the selection history or flaky-test records live only in that file, they vanish and the next run starts blind. Either persist the file deliberately, with care about concurrent restores, or accept that each run starts clean and design the logic that way. Mixing the two is the trap: code that quietly depends on history will behave differently on a fresh runner and on a long-lived one.
- Schema changes without a migration path. People add a column in code and forget that old database files exist on long-lived workers. The runner then fails on startup on some machines and not others. Every schema change needs a forward migration that runs on open, and it must tolerate being run against a file that is partly migrated.
- Treating the stored history as ground truth. The recorded pass and fail data is only as good as the runs that fed it. If a run was killed mid-way, it can leave a row that looks finished or looks stuck. Code that reads the history should expect half-written records and decide what to do with them, rather than trusting them.
- Not closing connections on the failure path. A crash in the middle of a run leaves a journal or a lock behind. The next start sees a database that appears busy. Make sure shutdown is handled on every exit path, including a cancelled job.

A related habit: people open the database by hand to look at things, and leave the shell open. That holds a lock. Use read-only access when inspecting, and close it.

## Getting GitHub Actions wiring wrong

verify-runner is usually triggered from a workflow. Most of the hurt is in how that workflow is set up, not in the runner.

- Pull request events from forks or from bot accounts have restricted permissions by default. The token may not be able to write a status, post a comment or read a secret. The runner finishes its work and then cannot report it, so the pull request shows nothing or a stale state. Check the permission scope of the workflow before blaming the runner. Do not widen permissions broadly to fix it. Grant only what reporting needs.
- Using an event type that runs with write access and secrets against code from the pull request. This is the classic way to hand credentials to untrusted code. If the workflow checks out the pull request head and runs install scripts in a context that has secrets, anyone who can open a pull request can read those secrets. Keep the part that executes upgrade code separate from the part that holds credentials, and pass only results between them.
- Concurrency. Upgrade bots open many pull requests close together, and each push to a branch can retrigger the workflow. Without a concurrency group that cancels superseded runs, the queue fills with runs for commits nobody cares about, and the runner's resources get spread thin. With a badly chosen group, a newer run cancels a different pull request's run and leaves it without a result. Group by pull request, not by workflow alone and not by branch name pattern that several bots share.
- Path filters and required checks. If the workflow only triggers on some paths and the check is marked as required, pull requests that skip it stay blocked forever, waiting for a check that never reports. If you filter, make sure a skipped case still posts a passing or neutral status that the branch protection accepts, and that it is clearly labeled as skipped.
- Timeouts. The default job timeout is generous, and a hung test can sit there burning minutes. Set an explicit limit on the verification step, and make sure the runner reports a distinct outcome for a timeout rather than letting it look like an ordinary failure or an ordinary pass.
- Matrix jobs. If verification fans out across several runtime variants, remember that the runner's own database and temp space are per job. Results need to be merged somewhere. People forget this and report only the last job that finished.
- Rerun semantics. Re-running a failed job reuses the same event payload, which may refer to a commit that is no longer the head. The runner then verifies something stale. When a rerun is requested, make sure the runner resolves the current state of the pull request, or at least records which commit it tested and shows that in the report.

## Misreading what a failure means

When verify-runner reports red, the instinct is to say the upgrade is bad. Often it is not. Sorting failures into kinds before reacting saves a lot of time.

- Flaky tests. A test that fails intermittently will fail on some upgrade runs by chance. Teams then close good upgrades, or worse, add the test to an ignore list and forget. Rerun once before concluding anything, but do not build an automatic retry-until-green loop. That hides real regressions, especially ones involving timing and concurrency, which are exactly what library upgrades like to introduce.
- Pre-existing failures. If the base branch was already red, every upgrade pull request is red too. The runner should compare against the base, and people should check the base first. A cluster of unrelated pull requests failing at the same moment points at the base or the environment, not at each upgrade.
- Infrastructure failures. A registry hiccup, a rate limit, a container that failed to start, a full disk. These produce a failed run that has nothing to do with the code. They should be classified separately and retried automatically, within limits. If they are lumped in with test failures, the pull request gets a misleading label and the human who looks at it wastes time.
- Environment-specific failures. A test fails only in the container because of a timezone, a locale, a file system case rule, or a missing tool. The upgrade was only the trigger that caused the test to run in this environment for the first time.
- Real regressions. These are the ones you want. They tend to be in a smaller set of tests close to the dependency's usage, and they fail the same way on rerun. The output you need is the failing test names and a short excerpt, not a wall of log.

Another mistake: reading only the summary. The runner's summary can say the run failed without saying which phase failed. Install, build, test selection, test execution and reporting are different phases. A failure in selection is a bug in the runner or its mapping. A failure in install is a dependency resolution or environment problem. Only a failure in execution is about the tests. Keep these phases visible in the output, and do not collapse them into one status.

## Selection logic and the upgrade-specific traps

The selector is the part most likely to be quietly wrong, so it gets its own section.

- Mapping from a package to files is usually derived by scanning imports. Barrel files and aliases break it. A project that re-exports a dependency through an internal module will show every consumer of that internal module as a consumer of the dependency, or none at all, depending on how the scan resolves it. Check how the scan treats re-exports for each repository type before trusting it.
- Monorepos. A dependency may be used in one workspace package and declared in another, or hoisted to the root. Selecting tests by the declaring package misses the consumers. Selecting by the whole repository defeats the point of targeting. The right answer needs the workspace graph, and the graph needs to be current. A stale graph, from a cache or from the base branch, produces the wrong subset.
- Test files that import the dependency directly are the easy case. Tests that exercise code which uses it are the hard case, and the selector only finds them by following the call or import graph outward. Decide how far outward to go, and be honest that the cutoff is a tradeoff between speed and coverage. Do not quietly raise or lower it per repository without recording why.
- Type-only usage. Upgrading a package that changes its types can break compilation without changing runtime behavior. A test run may not catch it if tests are transpiled without type checking. If the repository relies on type checking as part of its safety net, verification needs a type check step, not only tests.
- Lockfile-only changes. Some upgrades change only the lockfile, for example a patch-level update of an indirect dependency. There is no changed import to follow. The runner needs a rule for these that does not default to "nothing to run." What the rule should be is a policy question for the platform team, and it should be written down where reviewers can find it.
- Grouped upgrades. When several dependencies move in one pull request, the union of selections can be large, and a failure is hard to attribute. Bisecting by hand is slow. If grouping is used, the runner should at least report which dependency each selected test was chosen for, so a failure points back at a suspect.
- Major changes versus minor ones. The runner treats them the same unless told otherwise, but the risk is not the same. People forget to widen the selection when a change is flagged as breaking by the upstream project. Release notes are a human input here. The runner cannot read intent, so a person should know that a breaking change deserves a broader run.
- Generated code and snapshots. Upgrades that change generated output cause snapshot tests to fail. The fast fix is to regenerate snapshots, and doing that blindly accepts whatever the new version produced. Someone has to look at the diff of the snapshots and decide that the difference is expected.

## Operating habits that cause repeat problems

A few smaller things that don't fit above but keep recurring.

- Changing the runner and the repositories at the same time. If you alter selection logic and roll it out broadly in one step, you cannot tell afterward whether a change in pass rate came from the logic or from the upgrades. Roll out to a few repositories first and compare against what the old logic would have chosen.
- Logging too little or too much. Too little, and nobody can tell why a test was selected. Too much, and secrets and noise end up in logs that are retained for a long time. Log the decisions, meaning what was selected and why, and the phase outcomes. Do not log environment contents or raw install output with tokens in it.
- Ignoring disk and cleanup. Containers, images, temp checkouts and old database files accumulate on long-lived workers. The runner starts failing for reasons that look random, and the cause is a full disk. Clean up on exit and on a schedule, and make sure cleanup cannot remove something a concurrent run still needs.
- Skipping the dry run. Before pointing the runner at a new repository, run it against a known good state and a known bad one. If it passes the bad one, the selection is wrong for that repository. This takes little time and catches the mapping problems described above before they cost a reviewer's trust.
- Forgetting that trust is the product. Platform engineers who maintain many repositories will stop looking at the results the first time a green check on an upgrade turned out to be wrong, or a red one turned out to be noise three times in a row. Every shortcut that hides a failure or inflates a pass is borrowed against that trust. When in doubt, report less certainty rather than more. A result labeled as partial, skipped, timed out or unverified is far more useful than a clean-looking one that is not clean.
- Editing outcomes by hand. It is tempting to flip a status or insert a result to unblock a pull request. Do not. If the runner is wrong, fix the runner or mark the pull request as needing a manual check, so the record shows what happened.
- Assuming the next person knows the policy. Decisions about what counts as pass, which upgrades get a full run, and how empty selections are treated live in people's heads. Write them next to the configuration they affect, in plain words, and update them when behavior changes. Most of the confusion I have seen came from two people holding different unwritten rules about the same check.

If something above does not match what you see, trust what you see and fix this note. The component changes faster than the advice about it, and the failure modes listed here are the ones that were true when it was last looked at closely.
