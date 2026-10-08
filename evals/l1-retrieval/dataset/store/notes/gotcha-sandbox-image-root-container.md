---
id: 01KH8VYXWCF942AEXBP67C72AZ
created: 2026-02-12T09:07-03:00
---

# sandbox-image: non-root container fails on the npm cache directory

The sandbox-image runs fine as root and breaks as soon as the container is started as a non-root user, unless the npm cache directory is redirected somewhere that user can write. The failure is `EACCES: permission denied, mkdir '/root/.npm'`. It looks like a broken image but it is a configuration gap. Written down so nobody spends another afternoon on it.

## Symptom

A verification job in a non-root sandbox-image container dies early, during the dependency install step, before any targeted test runs. The log shows `EACCES: permission denied, mkdir '/root/.npm'`. PatchPilot then reports the upgrade pull request as unverified, not as a test failure. That is easy to misread as the upgrade being bad.

## Why it happens

npm picks its cache location from the home directory it believes it has. In the sandbox-image that home still points at root's, even when the process runs as a different user. npm tries to create its cache folder under root's home, the non-root user has no right to do that, and the mkdir fails. Nothing is wrong with the project being tested or with the lockfile.

## When it does not show up

If the container runs as root, the directory is created without complaint, so local runs that skip the user switch pass. Runs where a cache directory is already present and writable also pass. This is why the problem tends to appear only in CI, or only after someone hardens the image to drop root.

## How to confirm

Look at the first error in the install step output and check that it names root's home directory and the npm cache folder. Then check which user the container actually runs as. If it is non-root and the message matches, this is the gotcha. If the message names a different path, it is a different problem, so do not apply this fix blindly.

## Fix

Redirect the npm cache to a directory the non-root user owns or can create. Do this through npm's cache setting, either in the environment of the container or in an npm config file the image ships. Pick a location inside the working area of the sandbox, not inside root's home. The directory must exist or be creatable by the running user.

## Where to set it

Prefer setting it once in the image definition so every job inherits it. Setting it per job in the GitHub Actions workflow also works but drifts: a new workflow that forgets it brings the error back. If a job overrides the environment wholesale, check that the cache setting survives the override.

## Things that do not help

Retrying the job changes nothing, the failure is deterministic. Clearing caches does not help, since the issue is creation of the folder, not stale content. Running npm with more verbose logging only repeats the same message. Loosening permissions on root's home is the wrong direction and defeats the point of running non-root.

## Interaction with Docker layers

A cache directory baked into an image layer as root stays owned by root. If you create it at build time, make sure ownership is handed to the runtime user, otherwise the user can see the folder but still not write into it, and you get a similar permission error with a different path. Creating the directory at container start avoids the ownership problem.

## Interaction with mounted volumes

If the cache is placed on a mounted volume, the volume's ownership decides everything. A volume created by the Docker daemon as root will reject writes from a non-root process. Check the mount ownership before blaming npm. Sharing one cache volume between jobs can also cause lock contention, so keep it per job unless there is a reason.

## Effect on PatchPilot results

Because the install fails first, the targeted tests never start. The run record in the SQLite store ends up with a failed or errored status and no test output. When triaging a batch of upgrade pull requests, a cluster of identical install failures across many repositories points at the image or its environment, not at the dependencies themselves.

## Triage rule of thumb

If many unrelated repositories fail at the same step with the same message, suspect the sandbox-image first. If only one repository fails, look at that repository. This one is a many-repositories pattern, so start with the user and cache settings of the container.

## Testing the fix

After changing the image or the workflow, run a job as the non-root user against a small repository with a handful of dependencies. Confirm the install completes and the cache folder appears in the redirected location, not under root's home. Also run once as root to be sure nothing regressed for the old path of use.

## Related tools

Other tools inside the sandbox-image may also write under the home directory: package manager helpers, build caches, config folders. The same non-root problem can show up for them with their own messages. Fixing npm alone may just move the failure to the next tool, so run a full verification, not only the install step.

## Open questions

We have not decided whether the image should default to a non-root user. Doing so is safer but surfaces this class of error in every consumer that has not set the cache location. A middle path is to ship the redirect in the image and keep the user choice with the caller. Revisit when the hardening work is scheduled.

## Checklist for a new consumer

Check which user the container will run as. Confirm the npm cache is redirected to a writable place. Confirm any mounted volume is writable by that user. Run one install end to end before wiring the image into a larger batch of upgrade pull requests.
