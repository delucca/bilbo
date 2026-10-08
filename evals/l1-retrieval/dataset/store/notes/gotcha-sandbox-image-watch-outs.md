---
id: 01M1WVMEMQ9VCB5GP10MPAMX09
created: 2026-09-06T23:37-03:00
---

# sandbox-image: things to watch when changing it

The sandbox-image is where PatchPilot runs the targeted tests that verify an upgrade pull request. It is easy to treat it as just a Dockerfile, and that is how most breakage starts. Almost every change to it shifts what the tests see, and a shift in what the tests see shows up as a verdict on someone else's repository. A platform engineer looking at a red upgrade PR will blame the upgrade, not our image. So a bad change to sandbox-image does more than break our own jobs. It produces wrong results that look like real findings and cost other teams time.

These are notes from working on it, written quickly. They are general on purpose. Check the actual build files and the workflow definitions before relying on any detail here, because those are the source of truth and this note is not.

The short version: keep the image boring, keep it reproducible, keep it from reaching outside what it needs, and never change it and the orchestrator in the same breath without saying so in the review.

## What goes into the image, and what drifts

The biggest trap is silent drift. The image pulls in a base layer, a Node.js runtime, system packages, package managers, and sometimes language toolchains for repositories that are not TypeScript. Any of these can move underneath us if the build refers to a floating tag or an unpinned package. A rebuild next month can then produce a different image from the one that passed review, with nobody having touched the build file. Watch for this in particular:

- Base image references that point at a moving tag. A moving tag is fine for a quick experiment and wrong for the image that verifies other people's upgrades. Prefer a reference that cannot change without an edit in our repo.
- System packages installed without pinning. The package index moves, and a rebuild picks up whatever is current. If a package must float for security reasons, say so in a comment next to it so the next person does not pin it blindly and then forget it.
- Package manager versions. Repositories under test differ in which package manager and which lockfile format they use. If the image ships a package manager that is newer or older than a repository expects, installs can fail, or worse, succeed while rewriting the lockfile. A lockfile rewritten during verification makes the test run no longer match the PR diff.
- Global tools installed at build time. Anything installed globally is visible to every repository tested in the sandbox. A global tool can mask a missing dev dependency in a repository, so tests pass here and fail in that repository's own CI. Keep the global set as small as we can defend.

The opposite problem is just as real. If the image is too minimal, repositories with native addons fail to build. These need a compiler toolchain, headers, and sometimes extra libraries. When someone reports that a native module fails in the sandbox, check the image first, but do not reflexively add the missing piece. Ask whether the repository's own CI image has it, because we want to match what the repository's owners treat as normal, not become a superset of every environment.

Layer order matters for build time and for cache correctness. Put the things that change rarely early and the things that change often late. But also remember that a cached layer can hide a change. If a build step downloads something and the instruction text does not change, the cached layer is reused even though the upstream content has moved. When debugging a mystery, a build with the cache disabled is a cheap first move. When the mystery vanishes, the problem was a stale layer, and the right fix is to make that step depend on something that actually changes.

Image size is a real cost, not vanity. The image is pulled on every fresh runner in GitHub Actions unless a cache is warm, so each extra tool adds to every job's start time. Multi-stage builds help, but check that the final stage really contains what the runtime needs. It is common to drop a build-only dependency and discover later that a test run needed it after all.

Do not bake secrets into the image, and do not bake in repository checkouts. Build arguments end up in image metadata and history. A credential passed as a build argument is effectively published to anyone who can pull the image. Credentials the sandbox needs at run time must be injected at run time, scoped as narrowly as possible, and ideally not present at all while untrusted test code is running. See the next section.

## Behavior at run time inside the sandbox

The sandbox runs code from repositories we do not control, with dependency versions we have just changed. A new dependency version can run install scripts. So the image is the boundary between untrusted code and the runner, and changes that look like convenience can widen it. Be suspicious of anything that adds privilege.

- User and permissions. Running as a non-root user is the baseline. Adding root back to make an install step work is tempting and almost always wrong. Fix the ownership of the working directory instead. If a change needs elevated rights, find out which step needs them and whether it can happen at build time rather than run time.
- Writable locations. Tests write caches, temp files, and build output. If the root filesystem is read-only in some deployments and writable in others, a change that works locally can fail in the stricter one. Make sure every location the tools write to is either explicitly writable or redirected by configuration.
- Network access. Dependency upgrade verification needs the registry at install time, and often does not need the network afterward. If a change in the image alters proxy settings, certificate bundles, or DNS behavior, installs may work for public packages and fail for private registries, or the reverse. Corporate certificate authorities are a repeat offender: an updated base image may ship a different trust store, and private registry calls start failing in a way that looks like a network outage.
- Environment variables. Defaults baked into the image leak into every test run. A variable that sets a runtime mode, a locale, a timezone, or a memory limit can change test behavior in subtle ways. Locale and timezone in particular break date and sorting tests. Do not set these in the image unless there is a reason you can state in one sentence, and write the sentence down.
- Resource limits. Memory and CPU limits come from the orchestrator and the runner, not only the image, but the image's defaults interact with them. A runtime that sizes its heap from what it believes is available may behave differently depending on how the container reports its limits. When tests start getting killed with no clear failure, look at memory before looking at the test.
- Process handling. Test runners spawn children. If the container's init process does not reap or forward signals properly, a timeout kills the parent and leaves orphans, or the container hangs after the run is done. Changes to the entrypoint, the shell used for it, or the way the command is wrapped can break signal handling without any visible error. Check that a timed-out job really ends and really reports a timeout rather than a clean exit.
- Exit codes. The orchestrator turns the exit status of the sandbox into a verdict. A wrapper script that swallows a failure, or a shell option left unset so a failing step does not abort, converts a failing run into a passing one. This is the worst kind of regression because it produces approvals. Treat any edit to the entrypoint or wrapper as a change to the verdict logic and review it that way.

Clock and filesystem details also bite. Some tools compare modification times, and layered images can carry timestamps that make incremental builds in a repository skip work or redo it. Case sensitivity differs between the developer laptop and the container, so a repository with two files differing only in case, or an import with the wrong case, can pass on a Mac and fail here. That is a real finding about the repository, but make sure the report says where it ran.

SQLite deserves a note. PatchPilot keeps its own state in SQLite, and some repositories under test use it as well, often through a native binding. If the image ships a system library for it and a repository bundles its own, the two can disagree. Do not assume that the one the image provides is the one the tests use. Also, never put PatchPilot's own database inside the image or let the sandbox write to it. The sandbox should see a copy of what it needs or nothing at all.

## How the image connects to the rest of PatchPilot

The image is consumed by more than one thing: the orchestrator that picks the targeted tests, the GitHub Actions workflows that launch it, local development setups, and possibly users who run it by hand to reproduce a failure. A change that suits one consumer can break another, so list the consumers before editing.

Contract between the orchestrator and the image. The orchestrator expects the image to provide certain tools, to find the repository mounted or copied in a certain way, to use a certain working directory, and to report results in a certain form. These expectations are mostly implicit. When changing the working directory, the user, the shell, the entrypoint, or the location of results, search the orchestrator code for assumptions about each. The failure mode is usually not a crash. It is an empty result that the orchestrator interprets as no tests ran, which may be read as success or as a skip depending on the code path. Check what the orchestrator does with an empty result before shipping, and consider making that path loud.

Targeted test selection depends on the image indirectly. The selection logic reads the repository and the diff and chooses what to run. If the image changes which test runner or which version of it is available, the selection command may behave differently even though the selection logic is untouched. Watch for changes in how the runner reports names of tests, because the mapping from selection to execution often depends on name formats.

Tags and promotion. Be clear about which tag the workflows reference and how a new build becomes the one they use. If the workflows track a moving tag, then pushing a build is the same as deploying it to every job at once, with no gradual rollout and no easy rollback unless the previous build is still available under a known reference. Keep the previous good build around and written down somewhere a tired person can find it. If the workflows pin a reference instead, then a fix to the image is useless until the pin is bumped, and people forget the bump. Either way, say in the pull request which one applies.

Caching in Actions. Layer caching, dependency caching, and the image pull all interact. A cache key that does not include something the image changed can serve stale content into a new image, and the resulting failures are very confusing because the image and the cache each look fine in isolation. When the image changes in a way that affects installed dependencies, think about whether cache keys need to change with it.

Architecture. Runners and developer machines do not always share a CPU architecture. A base image or a prebuilt binary that exists for one architecture and not the other produces failures that only some people see. Emulation hides some of this and makes everything slower, which then trips timeouts. If a change adds a prebuilt binary download, check that it is selected by architecture and not hard-coded.

Docker on the host. The way the sandbox is launched, whether as a job container, a service, or a nested container started by a script, constrains what the image can do. Mounting the Docker socket into a sandbox to let tests build images is a serious privilege expansion and should not be added to make one repository work. If a repository needs it, that repository needs a different, deliberately separate path.

Documentation drift. The README or operator notes may describe the tools the image provides. When the contents change, those docs are usually wrong within a short time. Update them in the same change or delete the claims that are likely to rot.

## Testing a change, and rolling it out

A green build of the image proves almost nothing. It proves the instructions ran. What matters is whether the same repositories get the same verdicts before and after. The most useful habit is to keep a small set of known repositories with known outcomes, covering a clean pass, a real failure caused by an upgrade, a flaky test, a native addon, a private registry dependency, and a repository with a large lockfile, and to run them against the old and new image side by side. Compare verdicts, run duration, and the shape of the output, not only pass or fail.

Things worth checking by hand every time the image changes, even a little:

- A job that should fail does fail, and the failure reaches the pull request as a failure.
- A job that times out is reported as a timeout and the container is gone afterward.
- A job with no matching tests reports that clearly, and is not mistaken for a pass.
- The lockfile in the checkout is unchanged after a verification that was supposed to be read-only with respect to it.
- Private registry installs still work with the credentials as they are injected now.
- Output that the orchestrator parses still has the same format. Warnings from a newer tool version can land in a stream the parser reads.
- The image still starts quickly enough on a cold runner. Cold start is the number people feel.

Security updates are the sort of change people hurry. Rebuilding for a patched base is good, and it should still go through the comparison above, because patched system libraries have changed behavior before in ways that broke test suites. Do the rebuild promptly, but do not skip the check because the reason is urgent. The urgency argues for having the check be fast and routine.

When a verification result looks wrong, decide early whether the image or the repository is at fault. Reproduce the failure in the image outside of PatchPilot, using the same inputs, and then reproduce it in the repository's own CI environment. If it fails in both, it is likely a real upgrade problem. If it fails only in the sandbox, suspect the image: missing tool, different locale, different permissions, different network trust, different package manager. Record what you find. A recurring class of environmental difference is a sign that the image should change, and a one-off is a sign that the repository has an unusual need.

Rollback should be boring. Before shipping a change, know the way back and know that it works with the way workflows reference the image. A rollback that needs a code change in the workflows is slower than one that is a tag move or a pin revert. If a change touches both the image and the orchestrator, split it when possible so each can be rolled back alone. When it cannot be split, order the rollout so the orchestrator accepts both the old and new image behavior for a while, then remove the old path later.

Last, resist adding things for one repository. The pressure is constant: a team needs a tool, a font, a browser, a database client, a different runtime, and it is cheap to append a line. Each addition is paid for by every job, forever, in size, in attack surface, and in the chance that it masks a missing dependency elsewhere. Before adding, ask whether the repository can declare the need itself, whether an optional variant of the image is warranted, or whether the repository should not be verified by PatchPilot's sandbox at all. If you do add something, leave a short comment saying why and who needed it, so removal is possible later without archaeology.
