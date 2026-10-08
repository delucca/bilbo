---
id: 01KJNJ9DXJXXV0SQAJ3ZPNPMQV
created: 2026-03-01T17:43-03:00
---

# event-bus-bridge subscriptions spec

This note replaces the earlier note about "event bus bridge subscriptions". The new value: subscriptions in `event-bus-bridge` must use `ack_deadline_seconds=120`, where the old note said a deadline of 60 seconds.

## Naming

`event-bus-bridge` is the component name used everywhere in docs and config. Its internal codename is `buslink`. You will see `buslink` in older chat threads, some dashboards and a few log prefixes. Both names mean the same service. If you search the repo or the logs for one and find nothing, search for the other.

## What the component does

`event-bus-bridge` sits between Google Cloud Pub/Sub and the rest of FreightWeave. It pulls delay and status events from Pub/Sub subscriptions and hands them to the planner side, which uses OR-Tools to rebalance loads across truck and rail legs. It also publishes route-change events back out so dispatchers see updates. Redis holds short-lived state such as which messages were already seen. The HTTP surface is FastAPI, used for health checks and a few admin calls.

## Current requirement

Every subscription that `event-bus-bridge` creates or manages must set `ack_deadline_seconds=120`. This is the value to put in code, in any infrastructure definitions and in any test fixtures. The earlier deadline of 60 seconds is no longer correct and should not be copied from old examples.

## Why the change

Handling a delay event can include a re-solve of the affected routes. On busy regional corridors that work sometimes ran longer than the old deadline. The result was that Pub/Sub redelivered messages that were still being processed, which led to duplicate work and noisy rebalancing. A longer window gives the handler room to finish and ack before redelivery starts.

## Things to check when applying it

- Look for the old 60 second value in subscription creation code, in deployment definitions and in test setup. Change all of them, not just the main one.
- Existing subscriptions in deployed environments keep their old setting until someone updates them. Changing code alone does not fix those.
- Make sure any lease extension or modack logic in the consumer does not assume the old deadline.
- Keep the handler's own timeout shorter than the ack deadline, so a stuck solve fails and nacks before Pub/Sub redelivers.

## Interaction with Redis dedupe

The Redis seen-message markers should live at least as long as the ack deadline plus some margin. If the marker expires before the deadline passes, a redelivered message could be treated as new. Review the marker lifetime after the deadline change and raise it if it is too close.

## Gotchas

Older notes, tickets and comments may still describe the 60 second deadline. Treat them as out of date. Also, people who say `buslink` may be quoting those older sources, so confirm the deadline value before trusting them.

## Open points

- Whether all deployed environments have been updated is not recorded here. Check each one before calling this done.
- If the longer deadline slows recovery after a consumer crash in a way dispatchers notice, revisit it, but do not go back to the old value without a new decision written down.

## Where to update this note

Edit this note when the deadline changes again or when the codename is retired. Keep one note per subject for `event-bus-bridge`.
