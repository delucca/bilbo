---
id: 01M0TMA2008T1A5D727MNQY6ZP
created: 2026-08-24T16:35-03:00
---

# upgrade-scanner gotchas

Things that have bitten people, or will, when changing upgrade-scanner. It is the part of PatchPilot that looks at a repository, works out which dependencies are behind, and hands candidates to the pull request and test stages. It looks simple from outside. It is not, because everything downstream trusts what it emits. A small change in what upgrade-scanner reports shows up later as a wrong pull request, a missed upgrade, or a test run aimed at the wrong place. Read this before touching it, and add to it when you find something new.

## Output is a contract

Other stages read what upgrade-scanner produces and do not re-check it. If you rename a field, change its meaning, or change the order candidates come out in, you have changed a contract even if no type complains. Search for every consumer, including the ones that read rows back out of SQLite later and the ones in workflow files. Prefer adding a field over repurposing one. If you must change meaning, change the name too, so old readers fail loudly instead of quietly misreading.

## Scanning must not mutate the repository

The scanner runs against checkouts that other jobs may also be using. It should read files and nothing else. Avoid anything that rewrites lockfiles, creates caches inside the working tree, or leaves temp files behind. A helper that "just resolves" versions by running a package manager can write to disk as a side effect. Check what any new helper does to the tree, and compare the tree before and after in a test.

## Lockfile and declaration disagree

Declared ranges and locked versions often differ, and sometimes the lockfile is stale or missing. Decide explicitly which one counts as the current version for each ecosystem, and keep that choice in one place. Do not let one code path read the declaration and another read the lockfile for the same package. That produces candidates that claim to upgrade from a version the repository does not actually use.

## Monorepos and workspaces

One repository can hold many packages that share a lockfile, or that each have their own. The scanner has to attribute a dependency to the right package, not just to the repository. Mistakes here send the test stage to the wrong directory or produce one pull request where the team wanted several. When you touch discovery, test with nested workspaces, with a workspace that is excluded, and with a package that appears to be a workspace member but is not listed.

## The same dependency in several places

A dependency may appear as a runtime dependency, a development one, an optional one and a peer one, in more than one package. Deduplicating by name alone merges things that should stay apart. Not deduplicating creates duplicate pull requests. Pick the grouping key deliberately and write down why. Peer and optional entries deserve special care: bumping them can break consumers who are not in this repository.

## Version comparison is not string comparison

Prerelease tags, build metadata, and unusual schemes from some registries all break naive ordering. Use the same semver library everywhere in upgrade-scanner and do not hand-roll comparisons in one corner. Be wary of packages that do not follow semver at all, or that publish tags out of order. Never assume the highest-looking version is the newest or the intended one.

## Prereleases and yanked releases

By default the scanner should not propose a prerelease unless the repository is already on one. Yanked, deprecated or withdrawn releases must be filtered before they become candidates. If registry data for that is missing or partial, treat the candidate as unsafe rather than fine. A change that loosens these filters needs a test for each kind of bad release.

## Registry calls and rate limits

Many repositories mean many registry lookups. Cache within a run, batch where the registry allows it, and back off on throttling responses. Do not retry in a tight loop. A scanner that hammers a registry can get the whole installation throttled, which then stalls unrelated repositories. Make sure a failed lookup for one package does not abort the whole scan of a repository; record it and move on.

## Private registries and credentials

Some repositories use private registries or scoped sources. Credentials must never be logged, stored in SQLite, or placed in candidate records. Be careful with debug output when you add new logging: URLs can carry tokens. If a private source is unreachable, report the dependency as unscannable, not as up to date. Those two states look the same to a careless reader and mean opposite things.

## Ignore rules and pinning

Teams tell PatchPilot to ignore certain packages or to stay within a range. Those rules must be applied inside upgrade-scanner, before candidates are stored, and the same way every time. Check precedence when several rules match: a repository rule against an organization default, a package rule against a wildcard. A change to matching, such as case handling or scope prefixes, can silently start upgrading things people explicitly froze.

## SQLite state

The scanner records what it has seen so later runs can tell new candidates from known ones. Schema changes need a migration that works on existing databases, and must be safe if a run is interrupted partway. Keep writes for one repository in a single transaction. Watch for concurrent writers: two scans finishing together can hit locking, so decide how that is handled instead of hoping. Never assume the table is empty on start.

## Idempotence and reruns

Running the scanner twice on the same unchanged repository should give the same candidates and create no new work. Anything keyed on time, random values, or iteration order of a map will break this. Sort outputs. If a candidate was already turned into an open pull request, a rerun must recognize it rather than open another. Test the rerun case every time you change identity or keying of candidates.

## Interaction with open pull requests

Upgrade branches go stale as the base moves, and a newer version may appear while an older upgrade is still open. Decide in one place whether the scanner supersedes, updates or leaves alone the existing pull request. Do not let the scanner and the pull request stage each decide separately. Closed-without-merge pull requests are a signal that humans did not want that upgrade; do not blindly propose it again on the next run.

## Test targeting depends on scanner data

The targeted test stage uses what upgrade-scanner knows about which package, directory and files a dependency touches. If you make the scanner less precise, tests run too broadly and slow everything down. If you make it wrongly narrow, a breaking upgrade passes with no relevant test run. When changing how usage or ownership is detected, check both failure directions and keep a fallback that runs more, not less, when the data is uncertain.

## Running in GitHub Actions and Docker

The scanner runs in workflow runners and in containers, where the filesystem, the user, the time limit and the network differ from a developer machine. Do not depend on tools being installed on the host; confirm they are in the image. Watch memory on very large repositories and do not read whole trees into memory at once. Paths from the checkout may be mounted differently, so never bake in assumptions about where the repository lives.

## Logging and failure reporting

Errors need the repository and package context, or nobody can act on them. At the same time, a partial scan should be visibly partial in what it reports. A silent skip is worse than a loud failure because it looks like a clean result. Keep log lines stable enough that people who grep them in workflow output do not get broken by a wording change, and avoid dumping large payloads.

## Testing habits

Use fixture repositories that look like real ones: odd lockfiles, missing files, empty dependency sections, huge dependency lists, and files with unusual encoding or line endings. Mock the registry at the HTTP boundary, not deep inside, so retry and parsing code is covered. Avoid tests that need the live network. When you fix a bug here, add the fixture that triggered it, since the same shapes come back.

## Before you merge

Run the scanner over a handful of varied real repositories and diff the candidate output against the previous behavior. Read the diff, do not just count it. Unexpected additions mean looser filters; unexpected removals mean a bug or a changed rule. Tell downstream owners if anything about the output shape or ordering changed, even slightly.
