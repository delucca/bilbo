---
id: 01KWJD81B6XZ7TM95CTQ10EWF9
created: 2026-07-02T18:55-03:00
---

# sandbox-image: where it is published

Quick notes on how the sandbox-image gets built and pushed to the registry, written from memory of how the pipeline behaves. Not a full spec.

## What it is

The sandbox-image is the Docker image PatchPilot uses to run targeted tests for an upgrade pull request. It holds Node.js, the package managers, and the small set of tools the test runner expects.

## Where it lives

It is pushed to the container registry the org already uses for internal images. The repository name is the usual one under the platform namespace. Check the workflow file for the exact reference before relying on this note.

## Who publishes

A GitHub Actions workflow does the publishing. People do not push by hand except in an emergency, and if they do it should be noted in the team channel.

## Trigger

The workflow runs on changes to the image definition on the main branch, and also on a manual dispatch. Pull requests build the image but do not push it.

## Tags

Each publish gets an immutable tag tied to the commit, plus a moving tag for the current stable one. The runner config points at the moving tag by default. Pinning to the immutable tag is better when debugging.

## Credentials

The workflow logs in with a token stored as a repository secret. The token has push rights only to the sandbox-image repository. Rotation follows the normal schedule for registry tokens.

## Build context

The build is a plain Docker build with the Dockerfile at the root of the image directory. Layer caching is enabled in CI, so a base image bump is the thing that invalidates most of it.

## Base image

The base is a slim Node.js image. We follow the configured major line and update it when the runtime support window moves. Do not float the base without a pin.

## Pulling at runtime

The PatchPilot worker pulls the image before a verification run if it is not already cached on the host. Pull timeouts use the configured limit, and a slow pull shows up as a run that starts late, not as a test failure.

## SQLite link

Run records in SQLite store which image tag a verification used. That makes it possible to tell later whether a flaky result came from an image change.

## Gotchas

- The moving tag can change under a long-running batch, so two runs in one batch may use different images.
- A failed publish leaves the old stable tag in place, which can hide a broken build for a while.
- Local builds on Apple silicon may produce a different architecture than CI unless the platform is set explicitly.

## Rollback

To roll back, repoint the moving tag at a previous immutable tag. Do not delete tags; old run records refer to them.

## Cleanup

Old immutable tags are pruned by a retention rule in the registry. The retention period is whatever the registry policy says; ask the platform owners if you need an old one kept.

## Open questions

- Whether to sign images and verify on pull.
- Whether the moving tag should be dropped in favour of explicit pins in the runner config.
- Whether publishing should wait for a smoke test run against a sample repository.

## See also

The workflow file for the publish job, the Dockerfile in the image directory, and the runner config where the image reference is set.
