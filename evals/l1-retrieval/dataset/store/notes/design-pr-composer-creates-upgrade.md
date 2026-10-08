---
id: 01K77M7XDN82DDQMGKFPV1EG22
created: 2025-10-10T14:56-03:00
---

# pr-composer design

pr-composer is the part of PatchPilot that turns a chosen dependency upgrade into a pull request on GitHub. It does not decide what to upgrade and it does not run tests. It takes a finished upgrade plan, writes it to a branch, opens the pull request, and keeps that pull request tidy afterwards. This note records how it is put together and why, so the next person (or agent) touching it does not have to rediscover the reasons.

The short version: pr-composer creates upgrade branches named `patchpilot/<package>-<version>` through the `Octokit` REST client. Everything else in the component hangs off that choice. The branch name is how we find our own work again, and the REST client is how we talk to GitHub without needing a local clone for the common case.

## Scope and responsibilities

pr-composer owns four things. First, creating the upgrade branch from the base branch of the target repository. Second, committing the file changes that make up the upgrade, usually a manifest edit and a lockfile edit. Third, opening the pull request with a title and body that a reviewer can act on. Fourth, updating or closing that pull request when the situation changes, for example when a newer version of the same package appears before anyone merged the first one.

It does not own dependency resolution, which happens upstream and hands pr-composer a concrete package and a concrete target version. It does not own test selection or test execution, which are triggered by the pull request after it exists, through GitHub Actions. It does not own scheduling. Keeping these edges sharp is deliberate: when pr-composer fails, the failure is about GitHub state, never about whether the upgrade is good.

The component is used by platform engineers who maintain many repositories, so the design favours predictable behaviour across repositories over cleverness in any single one. A pull request from PatchPilot should look the same everywhere, and a reviewer who has seen one should know how to read the rest.

## Branch naming

Branches are named `patchpilot/<package>-<version>`. The prefix groups every branch we create under one namespace, which makes it easy to list, filter, and clean them up without touching human branches. The package and version parts make the name unique per upgrade target, so two different upgrades never collide and the same upgrade retried later lands on the same branch.

That last property matters. Because the name is derived from the package and the target version only, pr-composer can treat the branch as an idempotency key. If the branch already exists, we know a previous run got at least that far, and we inspect it instead of blindly creating another. This is how reruns after a crash stay safe.

Package names need care when they become part of a branch name. Scoped package names in the npm ecosystem contain characters that are awkward in refs, and other ecosystems have their own quirks. pr-composer normalises these characters into something valid for a git ref before building the name, and the normalisation is a pure function so the same input always yields the same branch. Do not hand-build branch names anywhere else in the codebase; always go through the one function in pr-composer, because the cleanup and lookup logic depends on exact matches.

Versions are used as the resolved target version, not a range. A range would make the name ambiguous and would break the lookup of an existing branch.

## Why the Octokit REST client

All GitHub calls from pr-composer go through the `Octokit` REST client. We chose it for three reasons. It is the maintained, typed client for GitHub, so the TypeScript types catch a lot of mistakes at compile time. It handles authentication styles we need, including installation tokens for a GitHub App and plain tokens for simpler setups. And its plugin model lets us add throttling and retry behaviour in one place rather than scattering it.

The REST approach also means pr-composer can create a branch, write file contents, and open a pull request without cloning the repository. For a platform team with many repositories, avoiding a clone per upgrade saves a lot of time and disk, which matters when Docker-based workers are short lived. The cost is that large or unusual changes are harder to express through the API, which is why there is a fallback described later.

We wrap `Octokit` in a thin internal interface rather than passing it around. The rest of pr-composer depends on that interface, which makes unit tests simple: tests supply a fake that records calls and returns canned responses. Nothing in the tests talks to the real network.

## Flow of a single upgrade

A run for one upgrade goes in a fixed order. pr-composer receives the repository, the package, the target version, and the file changes. It computes the branch name and asks GitHub whether the branch already exists. If it does not, it reads the base branch head and creates the new branch from there. It then writes the changed files to the new branch as one logical commit, and opens a pull request against the base branch.

After the pull request exists, pr-composer records what it did in the local SQLite database: which repository, which branch, which pull request, and what state it believes the pull request is in. That record is the source of truth for our own bookkeeping, but GitHub remains the source of truth for the pull request itself. On any disagreement we trust GitHub and repair the record.

Each step is written so that it can be repeated. Creating a branch that exists is treated as success after verification. Writing files that already match is a no-op. Opening a pull request when one is already open for the branch returns the existing one. This is what lets the scheduler simply retry a failed run without a special recovery path.

## Pull request content

The title states the package and the version change in plain words. The body is written for a reviewer who has only a minute: what changed, what the previous and new versions are, where the release notes can be found if we have a link, and a short note that tests were triggered automatically. We avoid long generated changelogs in the body, since reviewers skip them and they bloat the page.

Labels are applied so that the pull requests can be filtered by humans and by automation. The labels are configurable per repository, and pr-composer never creates labels that do not exist unless the repository settings explicitly allow it. A missing label is logged and skipped, not treated as a failure; a pull request without a label is still a useful pull request.

The body also carries a small machine-readable marker, so later runs can recognise a pull request as ours even if someone renames the branch. The branch name is the primary key, and the marker is the backup.

## Superseded and stale upgrades

Upgrades pile up. A package may release a new version while the pull request for the previous one is still open. pr-composer handles this by looking for open pull requests from branches under the `patchpilot/` prefix that target the same package. When a newer target version arrives, it opens the new pull request and then closes the older one with a comment pointing to the replacement. It closes rather than force-updating the old branch because the branch name encodes the version, and silently changing what a branch with an old version in its name contains would be confusing.

We do not delete the old branch immediately. Reviewers sometimes want to look at what it contained, and deleting is hard to undo. Branch cleanup is a separate, later step with a conservative delay and only for branches whose pull requests are closed or merged.

If a human has pushed commits to one of our branches, pr-composer notices that the head is no longer a commit it wrote and stops touching the branch. It leaves a comment instead. Overwriting human work is the one thing this component must never do.

## Errors, limits, and retries

GitHub rate limits and transient errors are the most common failures. Throttling and retry are configured on the `Octokit` instance, and pr-composer itself only distinguishes three outcomes: success, retryable failure, and permanent failure. Retryable failures go back to the scheduler with a delay. Permanent failures, such as missing permissions or a protected base branch that rejects the write, are recorded with a clear reason so that a platform engineer can see why a repository is being skipped.

Secondary rate limits are the nastiest case, because they punish bursts of write calls. pr-composer spaces its writes and does not fan out many repositories at once from a single token. If you add a new write call, think about how it behaves when many repositories are processed in a batch, even though you will probably only test it on one.

Error messages that reach logs must not contain tokens or full request headers. The wrapper around the client strips those before logging. Keep it that way when adding new calls.

## Testing approach

Unit tests cover branch name construction, normalisation of awkward package names, the decision logic for existing branches and pull requests, and the supersede behaviour. They use the fake client and run quickly. A small set of integration tests runs against a throwaway repository in a dedicated test organisation, exercised from GitHub Actions, and these check the real shape of API responses so the fake does not drift away from reality.

When GitHub changes a response in a way that breaks the fake, fix the fake and add a case to the integration tests in the same change. We have been bitten by fakes that were more forgiving than the real service.

Note for anyone debugging locally: the SQLite record can lag behind GitHub if a run died between the API call and the write. That is expected and gets repaired on the next run for the same branch name. Do not delete rows by hand to force a rerun; rerunning is already safe.

## Open questions and known gaps

Large changes, such as a lockfile rewrite that is too big for a comfortable API payload, are the main weak spot of the no-clone approach. The intended fallback is to do the work in a short-lived checkout inside a Docker worker and push the branch normally, keeping the same branch name so the rest of the flow does not care how the branch was produced. That path is only partly built, and any work on it should keep the naming and idempotency rules above unchanged.

Another gap is monorepos, where one package can be upgraded in several workspaces. Right now the branch name has no workspace component, so a single upgrade covering several workspaces becomes one branch and one pull request. Splitting them would mean changing the naming rule, which touches lookup, cleanup and the stored records, so it needs a deliberate design change and not a quick patch.

Finally, we have not settled how to present grouped upgrades, where several packages move together. For now each package gets its own branch. If grouping is added, decide the naming first, since everything else in pr-composer keys off it.
