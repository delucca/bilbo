---
id: 01KV10CGK9K65FQ77AM0DV28HH
created: 2026-06-13T14:27-03:00
sources:
  - "code: src/scanner/parse.ts"
---

# upgrade-scanner design

The upgrade-scanner parses package.json and package-lock.json and uses the semver library to separate patch updates from major updates. That one sentence is the core of the design: two manifest files in, a classified list of available upgrades out. Everything else in PatchPilot that opens pull requests or picks tests starts from what upgrade-scanner reports.

## Purpose

PatchPilot automates dependency upgrade pull requests for platform engineers who look after many repositories. The upgrade-scanner is the first stage. It looks at one repository checkout and says which dependencies could move, and whether each move is a patch update or a major update. It does not open pull requests and it does not run tests.

## Inputs

It reads package.json for the ranges the project declares, and package-lock.json for the versions actually installed. Both are needed. The declared range tells us what the maintainers allow. The lock file tells us what is resolved right now. Comparing only the range would miss drift, and reading only the lock file would lose the intent behind a pinned or loose range.

## Classification with semver

The semver library does the version comparison. We do not hand-roll string splitting. For each dependency, the scanner takes the installed version from package-lock.json and a candidate newer version, and asks semver how they differ. A change in the leading number is a major update. A change that only touches the last number is a patch update. Anything between those is kept as its own group and handled later, not forced into either bucket.

## Patch updates

Patch updates are the low-risk group. The scanner marks them so later stages can batch them and run a narrow set of tests. Pre-release tags are not treated as patch updates just because the last number moved.

## Major updates

Major updates are separated so they never get mixed into a patch batch. They tend to need a human to read release notes, and they usually need a wider test run. The scanner only labels them; policy about what to do with them lives elsewhere.

## Output

The result is a plain list of records: package name, current version, candidate version, and the class of change. It is stored for the next stage, using the SQLite database the rest of PatchPilot uses, so a rerun can tell what it already reported.

## Failure cases

If package.json and package-lock.json disagree badly, or one is missing or unparsable, the scanner reports that repository as unscannable instead of guessing. A malformed version string that semver rejects is skipped for that one dependency and logged, so one bad entry does not stop the whole scan.

## Running environment

It runs in Node.js, written in TypeScript, and is meant to work inside GitHub Actions jobs and inside Docker containers. It needs only the checked-out files, no network access to the registry for the parsing step itself.

## Open points

Workspaces with several package.json files are handled one at a time for now. Whether to merge them into a single report is undecided. Dependencies from git URLs or local paths have no useful version to compare and are left out.
