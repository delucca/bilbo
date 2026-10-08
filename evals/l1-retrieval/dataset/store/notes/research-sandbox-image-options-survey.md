---
id: 01JR922791DT8VNBQBN634VCE3
created: 2025-04-07T18:21-03:00
---

# sandbox-image: options survey

This is a survey of the options we have looked at for the sandbox-image, the container image in which PatchPilot runs the targeted tests for an upgrade pull request. Nothing here is settled. It is meant to save the next person from re-listing the same choices. The main tension is that upgrade PRs touch many different repositories, each with its own toolchain, while we want one image story that is small enough to maintain.

## What the image has to do

The sandbox-image runs untrusted code: whatever the upgraded dependency ships, including install scripts. So isolation matters more than speed of build. It also has to hold a Node.js runtime and a package manager, plus enough of the target repo's toolchain to run the narrow test selection. Some repos need native build tools, some need a database client, some need nothing special.

Other constraints that keep coming up:

- Startup time matters, because we run many small jobs rather than a few large ones.
- The image gets pulled by GitHub Actions runners, so pull size and registry location affect wall time.
- Network access during tests should be limited or at least observable.
- The SQLite state PatchPilot keeps should stay outside the image, so a sandbox crash cannot corrupt it.

## Option A: one fat general image

A single image with several Node.js versions, common compilers, git, and the usual package managers. This is the simplest to reason about and the simplest to cache on runners.

Upsides: one thing to build, scan and patch. Fewer surprises about which tools exist.

Downsides: it grows without limit as repos ask for more. Large pulls hurt every job, including the ones that need almost nothing. A vulnerability in any bundled tool shows up in scans for all jobs. It also hides which repos depend on which tool, so removing anything feels risky.

## Option B: thin base plus per-repo layers

A small base image with the runtime and git, and a derived image per repo or per repo family that adds what that repo needs. Derived images are built from a declared list of requirements kept near the repo config.

Upsides: pulls are smaller for simple repos, and the extra tooling is visible and reviewable. Scanning findings map to the repos that actually use the tool.

Downsides: more images to build and rebuild. Base image updates fan out into many rebuilds. We need a place to store the per-repo declarations and a policy for who owns them. Cache behavior on runners gets worse as the number of distinct images rises.

## Option C: build the environment at job time

Start from the thin base and install what the repo needs when the job starts, using the repo's own lockfile and a few declared system packages. No per-repo image at all.

Upsides: nothing to pre-build or store. The environment follows the repo automatically.

Downsides: every job pays install time, and failures in the install step look like failures in the upgrade. It also needs network access at the moment when we most want to restrict it, and it makes runs less reproducible, since system packages can move between two runs of the same PR.

A variant is to install at job time but through a read-through cache for packages. That cuts some cost, but it adds a service to run.

## Option D: reuse the repo's own container setup

Some target repos already have a Dockerfile or a dev container definition. We could run tests in that, and skip our own image for those repos.

Upsides: it is the environment the repo owners already trust. Little work on our side for those repos.

Downsides: quality varies a lot, and many repos have none. We would be running arbitrary build instructions from the repo, which widens what we have to trust. Hardening such as dropped privileges and read-only filesystems becomes a per-repo problem. The PatchPilot-specific tooling, such as test selection helpers, would still need to be injected somehow.

## Isolation and hardening choices

These apply whichever option above wins, and they are mostly independent of it.

- Run as a non-root user inside the container, with no extra capabilities, and a read-only root filesystem where the test runner permits it. Many test suites want to write to a temp directory, so that needs a writable mount that is thrown away.
- Network: either no network after dependencies are in place, or an egress allowlist. The first is stricter but breaks tests that call out. The second needs something to maintain the list.
- Secrets: keep none in the image, and pass none into test runs unless a repo has been explicitly set up for it. Tokens used to open PRs should live in a different step from the one that executes dependency code.
- Stronger runtimes such as a user-space kernel or microVM-based runners were considered as a hardening layer. They cost startup time and may not be available on hosted runners, so they stay a maybe.

## Build and update mechanics

Whatever shape we pick, the image needs a rebuild path. The ideas so far:

- Scheduled rebuilds so base OS patches arrive without anyone remembering to do it.
- Pin the base by digest in the build and update the pin through the same kind of automated PR that PatchPilot itself produces. That is a nice dogfooding case, but it also means a broken image can block the tool that would fix it, so a manual override is needed.
- Multi-stage builds to keep compilers out of the final image unless a derived layer asks for them.
- Image scanning in CI, with a policy on what blocks a publish and what only warns. We have not agreed on that policy.
- Tagging: immutable tags for each build plus a moving tag for the current one, so a bad build can be rolled back by repointing.

## Open questions

- How many distinct toolchain profiles do the managed repos really need? We have a rough feeling, not a count. A survey of the repos would tell us whether Option A or B is closer to the truth.
- Can GitHub Actions runner caching make a larger set of derived images cheap enough, or does it only work well for one or two?
- Is network-off feasible for the test selections we run, or do enough suites call out that an allowlist is unavoidable?
- Who owns per-repo tooling declarations: the platform team or the repo teams?
- Do we want a fallback to the repo's own container setup for repos with unusual needs, or do we refuse those repos?

## Leaning, not a decision

The current feeling is that a thin base with a small number of profile images sits between A and B and avoids the worst of each. Job-time installs of system packages look the weakest because of reproducibility. None of this is final, and it should be rechecked once we know how varied the managed repos are.
