---
id: 01JRDQ5QT0Z0R1XGC9XC05S5WY
created: 2025-04-09T13:47-03:00
---

# sandbox-image security review: tests ran as root

A security review of sandbox-image found that the container ran the target repository's tests as root. The conclusion was that the Dockerfile must end with `USER 1001`. This note records the finding, the reasoning, what the change touches, and what is still open. Nothing here has been applied yet. Treat it as the review result, not as a description of the current image.

## Outcome

The review verdict is that sandbox-image must not run tests as root. The last instruction of the Dockerfile has to be `USER 1001`, so every process started from the image by default runs as an unprivileged user. Root is acceptable during the build stages, where packages get installed and directories get prepared. It is not acceptable at runtime.

The review treated this as a required fix, not a suggestion. A later session should not reopen the question of whether to do it. The open questions are about how to do it without breaking upgrade verification, and those are listed further down.

## What sandbox-image is for

PatchPilot opens dependency upgrade pull requests and then runs targeted tests to check that each upgrade is safe. The tests run inside sandbox-image, a Docker image that holds the Node.js toolchain and whatever the test run needs. The code under test is a checkout of someone else's repository, and the upgraded dependency is new, third-party code that nobody on the platform team has read in full.

So the sandbox runs code with two untrusted layers: the repository's own test suite, and the freshly bumped package, including any install scripts it ships. That combination is why the privilege level of the process matters more here than in an ordinary build image.

## The finding

The image never switched away from root. Whatever user the base image defaulted to stayed in force, and that was root. Test commands, package install steps run at test time, and any lifecycle scripts of upgraded packages all executed with full privileges inside the container.

The reviewers noted that nothing in the image documented this as a choice. It looked like a default that nobody had revisited, not a deliberate decision to need root.

## Why root is a problem in this setup

A new version of a dependency can run arbitrary code the moment it is installed or imported. If that code is hostile, or if the package was taken over upstream, running as root inside the container gives it much more to work with than it needs.

- It can write anywhere in the container filesystem, including tool binaries that later steps trust.
- It can change ownership and permissions on mounted volumes, which leaves files on the host that the runner user cannot clean up.
- If a container escape bug exists, root inside the container is a much better starting point than an unprivileged user.
- Files created by root in a mounted workspace break the next job that reuses that workspace.

The last point is not a security issue but it showed up in the discussion as a practical reason to agree with the fix.

## Required change

The Dockerfile of sandbox-image must end with `USER 1001`. The instruction goes after all steps that need root, so it is the final user switch in the final stage. If the build uses several stages, it belongs in the stage that is actually shipped, not in a builder stage that gets discarded.

A numeric user is preferred over a name because it does not depend on an entry in the password file of the base image, and orchestrators that enforce non-root policies can verify a numeric id directly.

## What has to be prepared before the switch

Switching the user at the end only works if everything the tests need is already usable by that user. Before the final instruction, the build should make sure of the following.

- The working directory exists and is owned by the unprivileged user, or is writable by it.
- The package manager cache location is writable by that user, or is redirected to a place that is.
- The home directory the tools expect exists, since several Node.js tools write config and cache files there.
- Any temporary directory used by the test runner is writable.
- SQLite database files that PatchPilot shares with the sandbox, if any, are reachable with the permissions the unprivileged user has.

Ownership should be set during the build with the right copy options or an explicit ownership change before the user switch, not by running as root at start time and dropping later.

## Things likely to break

Some behavior will change once root is gone, and the review listed the likely failures so they are not a surprise.

- Tests that bind to low ports will fail, because an unprivileged user cannot do that. Those tests should use high ports.
- Tests that write to system paths, or install global packages at test time, will fail on permissions.
- Mounted workspaces created by the host runner may be owned by a different user than the one in the container, which shows up as permission denied on write.
- Repositories whose tests assume they can modify files outside the checkout will need to be flagged, not accommodated.

Failures of this kind should be reported as test failures with a clear permission message. They should not push anyone to widen privileges again.

## Workspace ownership with GitHub Actions

When the sandbox runs under GitHub Actions, the checkout is created by the runner and mounted into the container. The user inside the container has to be able to read and write that checkout. Two ways to handle it were discussed: make the checkout writable for the container user before the container starts, or let the runner create the container with a matching user setting. The first keeps the image simple and keeps the user switch in the Dockerfile as the single source of truth. The second spreads the decision across workflow files. The review leaned toward the first but left the final choice open.

## Verification of the fix

After the change, the image should be checked in three ways.

- Start a container from the image with no overrides and ask which user it is. The answer must be the unprivileged one.
- Run the normal targeted test flow against a sample repository and confirm that it passes without root.
- Try a write to a protected system location from inside the container and confirm it is refused.

The check on the effective user should become an automated step in the image build pipeline, so a later edit to the Dockerfile cannot quietly bring root back. A failing check should block publishing the image.

## Alternatives that were considered

Other approaches came up and were not chosen as the main fix.

- Dropping capabilities and keeping root. This reduces the damage but leaves the process as root, and it depends on every caller setting the flags. It can be added on top, not instead.
- Setting the user only in the workflow that launches the container. This leaves the image unsafe by default for any other caller, including local runs.
- Running a rootless container engine on the host. That is a good extra layer but it does not remove the need for the image itself to be safe.

The agreed position is that the image defaults must be safe, and runtime flags and host settings are extra layers.

## What the review did not cover

The review looked at the privilege of the test process in sandbox-image. It did not assess network access from the sandbox, secrets that might be passed into it, or the supply chain of the base image. Those are separate subjects and should get their own notes if someone looks at them. In particular, whether tests should have outbound network access at all was raised and left alone.

## Follow-ups

- Add `USER 1001` as the last user instruction in the Dockerfile and prepare ownership of the directories listed above.
- Add the automated effective-user check to the image pipeline.
- Run the targeted test flow on a few representative repositories and list the ones that fail on permissions.
- Decide how workspace ownership is handled under GitHub Actions.
- Write down in the repository docs that the sandbox is unprivileged by design, so nobody adds root back for convenience.

## Open questions

It is not settled how to treat repositories that truly need elevated rights for their tests. The current leaning is that PatchPilot reports them as not verifiable in the sandbox instead of running them with more privilege. That needs a decision from whoever owns the product behavior, since it changes what an upgrade pull request says about its own verification.

It is also not settled whether the unprivileged id should be configurable. The review preferred a fixed value in the image, for the reason that a configurable one invites a setting that resolves to root.
