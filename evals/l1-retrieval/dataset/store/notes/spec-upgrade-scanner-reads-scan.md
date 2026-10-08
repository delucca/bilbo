---
id: 01K49W0GFH4WJYMKRQ50YG580P
created: 2025-09-04T05:03-03:00
---

# upgrade-scanner spec: scan interval

This note specifies how the `upgrade-scanner` decides how often it looks for new dependency versions. It is short on purpose. It covers the one setting that controls the cadence, what the setting does, and what to check when scans seem to run too often or not often enough. How the scanner reads manifests and turns them into package entries is a different subject and lives in [[upgrade-scanner-parses-package]].

## The setting

The `upgrade-scanner` reads its scan interval from the environment variable `UPGRADE_SCAN_INTERVAL`. When the variable is not set, the default is `6h`. That means a scan cycle starts roughly every six hours, counted from the previous cycle, and not at fixed wall-clock times.

Nothing else sets the cadence. There is no second source for it: not a config file, not a per-repository override, not a command-line flag. If someone sees a different cadence than expected, the first thing to look at is the environment the process actually started with, not the code.

The value is read once, when the process starts. Changing the variable while the scanner is running has no effect. Restart the process, or recreate the container, to pick up a new value. This matters in Docker: editing a compose file or an env file does nothing until the container is recreated, and a plain restart of an existing container may keep the old environment.

Example of the setting, as it would appear in an env file or a container definition:

```
UPGRADE_SCAN_INTERVAL=6h
```

## What a scan cycle does

One cycle walks every repository that platform engineers have registered with PatchPilot. For each one it looks at the declared dependencies, asks the relevant registry whether newer versions exist, and records what it found in the SQLite database. Entries that are new or changed become candidates for an upgrade pull request. The pull request creation and the targeted test run are done by later stages, not by the scanner itself.

The interval is the gap between cycles, so it is a trade-off:

- A shorter interval finds new releases sooner, but it makes more registry requests and more writes to SQLite, and it can hit registry rate limits when many repositories are registered.
- A longer interval is gentler on registries and on the database, but a release can sit unnoticed for a while, and security fixes reach reviewers later.

The default `6h` was picked as a middle point: a new release is normally seen within the same working day, and the registry load stays modest even with a large number of repositories. Teams that maintain many repositories should think twice before shortening it.

## Operating notes

Where the variable should be set depends on how the scanner is deployed.

- In Docker, put it in the container environment. It should be part of the image's deployment definition and not baked into the image, so the same image can run with different cadences.
- In GitHub Actions, put it in the `env` block of the job or step that starts the scanner. A scheduled workflow has its own schedule trigger, and that is a separate thing from this variable. If both exist, the workflow schedule decides when the process starts, and `UPGRADE_SCAN_INTERVAL` decides the gap between cycles inside a long-running process. Do not assume they line up.
- For local development, export it in the shell before starting the process, or leave it unset and accept the default.

Format of the value: it is a duration with a unit suffix, in the same style as the default `6h`. Stick to that style. If a value cannot be understood, check the startup log for a complaint before assuming the default was used. Do not rely on silent fallback; if you see the cadence stuck at the default after setting the variable, the likely cause is a value the scanner rejected or an environment that never reached the process.

## Gotchas

- Setting the variable in the wrong place is the usual cause of "it ignores my interval". Confirm it inside the running container or job, not just in the file you edited.
- The interval is not a guarantee of an upgrade pull request at each cycle. A cycle that finds nothing new produces nothing.
- A scan that takes a long time can delay the next one. The interval is measured between cycles, so a slow registry stretches the real cadence beyond the configured value.
- Shortening the interval for a test and forgetting to restore it is easy to do in a shared deployment. Write down any temporary change.
- Tests that depend on timing should not wait for the real default. Set a small value in the test environment instead.

## Open points

There is no validation of a minimum interval that is documented here. If rate limit problems show up, a floor on the value would be a reasonable change, but it has not been decided. If that changes, update this note and keep the default stated in one place.
