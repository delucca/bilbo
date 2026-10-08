---
id: 01K4WD537BHPMS2M6R6525ZPAW
created: 2025-09-11T09:49-03:00
---

# event-bus-bridge spec

This is the working spec for event-bus-bridge, the piece of FreightWeave that moves events between Google Cloud Pub/Sub and the rest of the system: the FastAPI services, the Redis state and the OR-Tools rebalancing worker. Written quickly, so it covers what we need to build and operate it and leaves detail out where it is not settled.

The one hard rule so far: the event-bus-bridge subscriptions must use `ack_deadline_seconds=60`. Everything below either explains why or works around it.

## Purpose

Dispatchers need delay information to reach the planner fast. Carriers, yard systems and rail operators publish events. event-bus-bridge subscribes, normalizes them, and hands them to the route planner so loads can be rebalanced. It also publishes planner results back out so other consumers see the new plan.

## Scope

In scope: subscribing to inbound topics, validating and normalizing messages, deduplicating, passing work to the planner, publishing outbound plan-change events, acking or nacking correctly.

Out of scope: the planning logic itself (OR-Tools lives in the worker), the dispatcher-facing API, and any persistence beyond short-lived dedup and bookkeeping keys in Redis.

## Subscription settings

Every subscription owned by event-bus-bridge must be created with `ack_deadline_seconds=60`. Do not rely on the library default, and do not set it per subscriber in code differently from the subscription definition. If someone creates a subscription by hand for testing, it should use the same value, or behavior seen in test will not match production.

Why 60 and not shorter: handling a delay event can include a Redis lookup, a validation pass and a handoff to the planner queue. Under load that takes longer than a short deadline allows, and Pub/Sub then redelivers messages that are still being processed. That produced duplicate rebalances early on.

Why not longer: a crashed consumer holds its messages until the deadline expires. A long deadline means slow recovery after a crash, which matters when dispatchers are waiting on a reroute.

## Ack and nack behavior

Ack only after the event has been handed off durably to the planner path and the dedup marker is written. Never ack on receipt.

Nack on transient failures such as Redis being unreachable or the planner queue refusing work. Let Pub/Sub redeliver.

For malformed messages that can never succeed, log them with enough context to trace the source, then ack so they do not loop. If a dead-letter topic is configured on the subscription, prefer routing there over silent drops.

If handling is expected to run close to the deadline, extend it explicitly rather than hoping to finish. Treat any handler that regularly needs an extension as a bug to look at, not as normal.

## Deduplication

Delivery is at-least-once, so duplicates will happen even with the deadline set correctly. The bridge keeps a short-lived marker in Redis keyed by the message's business identifier plus event type, and skips work when the marker exists. The marker lifetime must be comfortably longer than the redelivery window implied by the ack deadline. Keep the exact lifetime in config, not in code.

Ordering is not guaranteed. Events carry a source timestamp, and the planner should prefer the newer state when two events about the same shipment arrive in reverse order.

## Failure modes

Redis down: nack, back off, alert. Do not process without dedup.

Planner worker slow: queue depth grows. The bridge should apply flow control on the subscriber so it does not pull more than it can hand off.

Pub/Sub outage: nothing to do but wait; the bridge should reconnect on its own and not require a restart.

Poison messages: handled as above. Watch for a spike, which usually means an upstream format change.

## Configuration

Subscription names, topic names, flow control limits and the dedup marker lifetime come from environment-based config. The ack deadline is the exception: it is part of the subscription definition and fixed at the value above, so changing it is a spec change, not a config tweak.

## Observability

Log each message with its business identifier, event type, delivery attempt and outcome (acked, nacked, dropped). Track counts of redeliveries and nacks. A rising redelivery count with no failures usually means handlers are too slow for the deadline.

## Testing

Unit test the normalizer and the ack decision logic with fake messages. For integration, use the Pub/Sub emulator with subscriptions created using the same deadline as production. Include a test that delivers the same message twice and checks that only one handoff happens.

## Open questions

Whether to add a dead-letter topic for every subscription or only the noisy ones. Whether outbound plan-change events need their own ordering key. Neither blocks the current work.
