---
id: 01KFC3NF272Z6G6J2TGCK76JJG
created: 2026-01-19T18:48-03:00
sources:
  - "doc: sandbox release runbook"
---

# sandbox-image reference

Quick reference for sandbox-image, the container image PatchPilot uses to run targeted tests against dependency upgrade pull requests. Written fast; check the registry before relying on anything beyond what is stated here.

## Name and codename

The component is called sandbox-image. Its internal codename is `boxbase`. If you see `boxbase` in chat, old tickets or internal scripts, it means sandbox-image.

## Registry path

sandbox-image is published to the registry path `ghcr.io/patchpilot/sandbox:stable`. That is the GitHub Container Registry, under the patchpilot organization, with the `stable` tag.

```bash
docker pull ghcr.io/patchpilot/sandbox:stable
```

## What it is for

It is the isolated environment where PatchPilot runs the tests that verify an upgrade. The goal is that a bad dependency bump cannot touch the host or the runner.

## Where it runs

Docker runs it. In CI, GitHub Actions jobs pull it and start the test run inside it.

## Tag meaning

The `stable` tag is the one consumers should pull. Treat other tags as not for general use unless someone tells you otherwise.

## Contents, in general terms

It holds the Node.js and TypeScript toolchain needed to install dependencies and run test commands for the target repository. Details of the exact versions are not recorded here.

## Relation to SQLite

PatchPilot keeps run state in SQLite on the orchestrator side. The image itself is not where that state lives.

## Who uses it

Platform engineers maintaining many repositories rely on it indirectly, through PatchPilot upgrade runs.

## Pulling and caching

Runners pull from the registry path above. Caching of layers is up to the runner; nothing special is set up for it that I know of.

## Updating the image

Changes to sandbox-image get published to the same `stable` tag. Anyone changing it should expect all repositories using PatchPilot to pick the change up on their next run.

## Gotcha: codename confusion

Searching the code or docs for sandbox-image may miss files that only say `boxbase`. Search for both.

## Gotcha: moving tag

Because `stable` is a moving tag, a run today and a run next week may not use identical contents. Keep this in mind when a test result changes without a code change.

## Debugging a failed run

First confirm which image contents the run actually used, then reproduce locally by pulling the same path and running the failing test command inside it.

## Open questions

- Exact build pipeline and who owns it.
- Whether pinned digests are used anywhere.
- Size and layer layout.

## Sources of truth

The registry page for the path above is the authority on what is published. This note only records the name, codename and location.
