---
id: 01KXT0YAH2FAZ8Z266RGJ2JW1A
created: 2026-07-18T04:09-03:00
---

# ci-dispatcher delivery dedupe window

The ci-dispatcher now drops repeated X-GitHub-Delivery events for 30 minutes, which replaces the earlier window of 10 minutes, because retries were observed up to 25 minutes late. This note replaces the earlier note "ci dispatcher drops repeated"; the value to trust is 30 minutes.

## Decision

The dedupe window in ci-dispatcher is 30 minutes. A second event carrying an X-GitHub-Delivery id already seen inside that window is dropped and no CI run is started for it. The old window was 10 minutes and is no longer correct anywhere.

## Why the old window was too short

GitHub redelivers webhooks when our endpoint is slow or returns an error. We saw retries arrive up to 25 minutes late. With a 10 minute window, those late retries were no longer recognised as repeats, so the ci-dispatcher treated them as new events and started the same test run twice.

## Why 30 minutes

The longest late retry seen is 25 minutes. 30 minutes covers it with a small margin and does not hold ids much longer than needed. We did not pick a larger value, so memory and table size stay modest.

## What counts as a repeat

Only the X-GitHub-Delivery id is compared. Same pull request, same commit, different delivery id: that is a new event and it is processed normally. Two events with the same delivery id are one event.

## Effect on pull request verification

Duplicate runs of targeted tests on one upgrade pull request should stop. Platform engineers who maintain many repositories should see fewer repeated check runs and less wasted Actions time.

## Storage

Seen delivery ids are kept in SQLite. Entries older than the window are expired and can be removed. Changing the window does not need a schema change.

## Config shape

The window is expressed in minutes. Illustrative setting:

```
dedupe_window = 30 minutes
previous = 10 minutes
```

## Risks

A genuine redelivery meant to force a rerun, sent inside the window with the same id, is dropped. To rerun, trigger a fresh event instead of a redelivery. Retries later than the window would still slip through.

## Follow-ups

- Check the Docker image of ci-dispatcher picks up the new value after deploy.
- Watch for retries later than 25 minutes and revisit the window if they show up.
- Make sure any docs or runbooks that mention the 10 minute window are corrected.

## Status

Decided and in effect. This note supersedes the earlier one on the same subject.
