---
id: 01JY0J5M7NV9QABJ1MSVJ4VVR3
created: 2025-06-18T00:43-03:00
---

# sandbox-image: overall structure

This is a working note on `sandbox-image`, the container image PatchPilot uses to run the targeted tests that verify an upgrade pull request. It describes how the pieces fit together, not how any value is set. Check the Dockerfile and the workflow files for current details, because this note will drift from them.

Short version: `sandbox-image` is a Docker image. A GitHub Actions job starts it, a checkout of the target repository goes inside, the dependency change is applied, and the tests that touch the changed packages run. The results go back to PatchPilot, which decides what to write on the pull request. The image holds the tools and the conventions for how a run behaves. It does not hold the target repository's code, and it does not hold PatchPilot's own orchestration logic.

## Purpose and boundaries

PatchPilot opens upgrade pull requests across many repositories. Opening them is the easy part. The hard part is knowing whether an upgrade breaks anything, and finding out without running untrusted dependency code on a machine that matters. `sandbox-image` is the answer to that second problem. Upgraded packages can run install scripts and test code, so that code runs in a throwaway container with limited reach.

What the image is responsible for:

- Providing a Node.js runtime and the TypeScript toolchain that most target repositories need.
- Providing the package manager tooling to install and resolve the upgraded dependency set.
- Providing SQLite so that repositories whose tests touch a local database can run without extra setup.
- Providing a small entrypoint that PatchPilot calls with a description of what to run.
- Keeping the run contained: a non-privileged user, a clean working area, and no long-lived credentials baked in.

What the image is not responsible for:

- Choosing which tests to run. The test selection part of PatchPilot does that and hands the result to the entrypoint.
- Deciding whether a result is good enough to merge. The image only reports what happened.
- Storing history. PatchPilot keeps its own SQLite state outside the sandbox.
- Talking to GitHub as PatchPilot. The job that wraps the container does that.

If a change needs something from the second list, it probably belongs somewhere else. Putting it in `sandbox-image` makes the image harder to reason about and harder to replace.

The image is deliberately boring. Platform engineers who maintain many repositories will read its output often and rarely read its code, so predictable behaviour matters more than clever behaviour. When a run fails they want to know quickly whether the upgrade is at fault or the sandbox is. Most design choices below come back to making that distinction easy.

## Layer structure

The image is built in layers, ordered from least to most frequently changed so Docker cache reuse works. Rebuilds should be cheap when only the entrypoint changes. Roughly:

```
sandbox-image
  base:    Node.js runtime
  tools:   TypeScript toolchain, SQLite
  runner:  entrypoint used by the GitHub Actions job
```

The base layer is the Node.js runtime on a slim Linux userland. It changes rarely and only on purpose. The tools layer adds the compilers, package managers and native libraries that dependency installs commonly need, plus the SQLite library and its command line tool. The runner layer is the thin top: the entrypoint, any helper files it reads, and the user setup.

The point of keeping the runner layer thin is that it changes the most. If someone adds a tool to the runner layer because it is quick, the cache benefit disappears for everyone. Put new system packages in the tools layer, even when that feels far from where they are used.

The build uses a multi-stage Dockerfile. An earlier stage compiles the TypeScript entrypoint and its helpers, and the final stage copies only the compiled output. This keeps build-only dependencies out of the image that runs untrusted code, and it keeps the image smaller, which matters because runners pull it often.

The compile stage and the final stage should share the same Node.js base so native modules built in one work in the other. Mixing them has produced confusing load failures. If a native module fails to load inside the sandbox, check that the bases match before looking anywhere else.

Some things deliberately do not live in any layer:

- Target repository code. It arrives at run time through the checkout.
- Registry credentials. They arrive at run time and only for the install step.
- Caches of dependencies from earlier runs. A cache would make results depend on history, and the sandbox is meant to have none.

## Entrypoint and result format

The entrypoint is the single place PatchPilot talks to the container. It takes a run description, does the work, and writes a result. Keeping it as the only interface means the rest of the system does not need to know which tools exist inside.

The flow inside the entrypoint, in order:

- Read the run description. It names the target repository checkout, the dependency change to apply, and the tests selected for this run.
- Prepare a clean working area. Anything left from an earlier run in the same container is removed or ignored. Containers are meant to be used once, but the entrypoint does not assume that.
- Apply the dependency change through the repository's own package manager, not by hand-editing manifest or lockfile.
- Install dependencies. This is where install scripts of the upgraded packages run, so it is the riskiest step and the one where the container limits matter most.
- Run the selected tests with a time limit. The limit is a guard against hangs, not a performance target.
- Collect output: the outcome of each step, test results in structured form, and the tail of the raw logs.
- Write the result where the job expects it and exit with a status that reflects the overall outcome.

Each step records its own outcome separately. A failed install and a failed test are different facts, and PatchPilot treats them differently. A failed install usually means the upgrade is incompatible or the registry had trouble. A failed test points at a behaviour change. Do not collapse them into a single flag, even when that would simplify the reader.

There is also a third kind of outcome that is easy to forget: the run could not happen. The checkout was missing, the run description was malformed, the tool needed was absent. That is neither an upgrade failure nor a test failure, and the result should say so with its own status. Otherwise a sandbox problem shows up on a pull request as if the upgrade were bad, and reviewers lose trust in the whole system.

The result is a structured document, not free text. It carries a status per step, a list of test outcomes, and some log excerpts for humans. The format is shared with PatchPilot's reader code, so treat it as a contract between the two. If you need to add information, add a field and keep the old ones. If a field is removed, check the reader and any stored results that still use it.

The log excerpts are for people. Nothing should parse them. If the reader needs a fact, that fact gets a field.

## How the job uses the image

The job runs the image as a container, mounts the checkout and the run description, and lets the entrypoint do its work. The job handles what the image should not: fetching the repository, passing in a scoped token when one is needed, uploading the result, and posting status back.

Points worth remembering:

- Secrets stay on the job side. The image has no credentials in its layers, and the entrypoint receives only what the run needs. When a private registry is involved, access is given for the install step only and is not written to disk inside the image.
- The job decides resource limits and network settings. The image can assume less than full network access, but it should degrade with a clear message when the network is missing and not hang. Tests that need the network are reported as such rather than silently skipped.
- The job owns the outer time limit. The entrypoint has inner limits per step, but a hung container is the job's problem to cut off, and the entrypoint should not rely on being able to clean up after itself when that happens.
- The job is responsible for getting the result out even when the entrypoint fails badly. A missing result is treated as a sandbox failure, not as a pass.

When debugging a failed run, start from the job log, then the result document, then the raw log excerpts. Most problems are visible in the step statuses of the result without reading raw output.

## Isolation and the SQLite question

The image is a boundary, so its setup matters more than its size. The measures are general and should stay that way:

- The entrypoint drops to a non-privileged user before running anything from the target repository.
- The root filesystem is treated as read-only where possible, with a writable area for the checkout and temporary files.
- No container runtime socket is exposed inside the sandbox.
- Outbound access is limited by the job configuration, not by tools in the image.
- Output is size-limited before it leaves the container, so a noisy test cannot flood the job log or the result.

This is defence in depth, not a guarantee. A hostile package could still try things. The assumption is that the container is disposable and that nothing valuable is reachable from it. If a change would put something valuable inside the sandbox, such as a long-lived token, stop and rethink the change.

SQLite appears in different roles and they are easy to confuse. PatchPilot's own state, such as which pull requests exist and what past runs concluded, lives in its SQLite database outside the sandbox. Inside `sandbox-image`, SQLite is included only because many target repositories use it in their tests, either through a native binding or through the command line tool.

Because of that, the sandbox should never need to reach PatchPilot's database. If a feature seems to need it, the data should be passed in through the run description, and anything that must be recorded should travel back in the result. Keeping the flow one-directional means the sandbox stays replaceable and its runs stay reproducible.

Native SQLite bindings are a frequent source of install-time trouble: they either download a prebuilt binary or compile from source. The tools layer carries what compiling needs, so both paths work. When a prebuilt download is blocked by network limits, compiling is the fallback, and that is why compilers stay in the image even though they enlarge it.

## Build and update flow

The image is built and published by a GitHub Actions workflow in the PatchPilot repository. Changes to the Dockerfile or the entrypoint source trigger a build. Published images are tagged so PatchPilot can pin which one it uses, and the pin is changed deliberately instead of floating to the latest.

A reasonable order when changing the image:

- Change the Dockerfile or entrypoint locally and build it with Docker.
- Run the entrypoint against a small sample repository with a known-good upgrade and a known-bad one. Both should report sensibly: the good one passes, the bad one fails at the expected step.
- Check that the result document still parses with the current reader.
- Push and let the workflow build and publish.
- Move the pin in PatchPilot to the new image in a separate change, so it can be reverted without touching the image.

Base updates for the Node.js runtime deserve extra care. They affect every target repository at once, and a repository that passed before can fail for reasons unrelated to the upgrade under test. The result then looks like an upgrade problem and wastes reviewer time. Prefer to roll a base update out gradually and watch a few runs before treating the new image as the default.

The same caution applies to the tools layer. Removing a tool is riskier than adding one, because the repositories that need it are spread across the whole fleet and nobody has a list.

## Rough edges and where to look

General observations, not tracked bugs:

- Target repositories differ in how they install and test. The entrypoint handles the common package managers. Unusual setups need either a hook in the run description or a fallback that reports "could not run" clearly. A clear refusal beats a confusing failure.
- Image size creeps up as tools are added. Nobody removes tools because nobody is sure who needs them. Before adding something, look for an existing tool that does the job.
- Log excerpts can cut off in the middle of a line. Readers should not assume the last line is complete.
- Time limits are coarse. A slow but healthy test run and a hung one look the same until the limit hits.
- The sandbox has no memory of earlier runs, by design. Anything that needs history, such as flaky test detection, has to be done on the PatchPilot side from stored results.

Where to start when changing things:

- What tools exist: the tools layer of the Dockerfile.
- What happens during a run: the entrypoint and its step order.
- How the run is launched, limits, or secrets: the workflow that runs the container, not the image.
- What PatchPilot does with a result: the reader, not the entrypoint.

Keep the split clear. The image provides tools and a contract, the job provides the environment and secrets, and PatchPilot provides decisions. Most bad changes to `sandbox-image` come from one of these taking over another's work.
