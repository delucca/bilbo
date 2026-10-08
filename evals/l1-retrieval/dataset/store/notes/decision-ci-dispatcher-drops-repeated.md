---
id: 01KDP2TVRF58VTM31G31HKS6X7
created: 2025-12-29T19:14-03:00
---

# ci-dispatcher drops duplicate webhook deliveries

The ci-dispatcher ignores a repeated webhook event when it carries the same `X-GitHub-Delivery` header as one it already saw in the last `10 minutes`. GitHub retries deliveries that time out, so the same event can arrive more than once. Without this, one event could start the same verification run twice.

## Decision

Treat `X-GitHub-Delivery` as the identity of an event. If the ci-dispatcher has seen that value within `10 minutes`, it drops the event and does nothing else. A first-seen value is processed as normal.

## Why

GitHub resends a delivery when the first attempt times out. The first attempt may still have been accepted and be running on our side. The retry then looks like a new request, and the ci-dispatcher would dispatch the targeted tests for the same upgrade pull request a second time.

## What it prevents

- Duplicate GitHub Actions runs for one upgrade pull request.
- Duplicate status updates and comments on the pull request.
- Wasted runner time when platform engineers maintain many repositories.

## Window

The window is `10 minutes`. It is meant to cover the retry period for a timed-out delivery. After the window, a repeated id is treated as new. That is acceptable because a retry should not arrive that late.

## Where the ids live

Seen ids can be kept in the SQLite database the service already uses. Each row holds the delivery id and the time it was first seen. Old rows can be pruned once they are past the window. A restart of the Docker container should not reset the dedupe state, which is why memory alone is not enough.

## Check flow

```text
receive event
read X-GitHub-Delivery
seen within 10 minutes? -> drop
otherwise -> record id, dispatch
```

## Things to watch

- Record the id before dispatching, not after. Otherwise a slow dispatch lets a retry slip through.
- A missing header is not a duplicate. Handle that case separately and do not drop on an empty value.
- Dropping must be quiet and cheap. A debug log line is enough.

## Not decided

Whether to keep the id longer than the window for auditing is open. For now only the dedupe behavior matters.

## Notes for later work

If the retry behavior of GitHub changes, revisit the window length first. Tests for the ci-dispatcher should send the same `X-GitHub-Delivery` value twice and expect one dispatch.
