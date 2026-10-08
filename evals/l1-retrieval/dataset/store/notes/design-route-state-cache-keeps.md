---
id: 01JRPHEQBX0X8RJKRWTDJY9WCT
created: 2025-04-13T00:00-03:00
sources:
  - "code: freightweave/cache/keys.py"
---

# route-state-cache design

The route-state-cache holds the live state of each route that FreightWeave is currently planning or running. It keeps that state in a Redis hash under the key pattern `route:{route_id}:state`. There is one hash per route, and the route identifier is the only thing that picks the key. This note records how the cache is shaped, who writes to it, who reads from it, and where it can go wrong. It is written from the design as it stands, not from a measured deployment, so tuning values are left out on purpose.

## Purpose and scope

Dispatchers need to see where a multi-leg route stands right now. The planner needs the same picture when a delay shows up and loads have to be rebalanced. Both need a fast read that does not wait on a full recompute. The route-state-cache is that read path. It is not the system of record for plans. The plan, as produced by the OR-Tools solver, lives in the planning side of the system. The cache holds the current, operational view of a route: which leg is active, what has been completed, what is late, and which assignments are in force.

The cache is deliberately small in what it promises. It answers the question of what is true about this route at this moment. It does not answer what was true earlier, and it does not answer what the best plan would be. History belongs to other stores, and optimisation belongs to the solver.

Things the route-state-cache is not for:

- Long-term audit of route changes.
- Holding solver inputs or intermediate solver output.
- Acting as a queue. Events travel through Google Cloud Pub/Sub, not through Redis.
- Sharing data between unrelated routes. Each route is its own island.

## Key layout

Every route gets exactly one hash, and the key pattern is `route:{route_id}:state`. The braces in the pattern are part of the pattern as written, so the route identifier sits inside them. This also matters if the deployment ever moves to a clustered Redis, because the braced part is what a cluster would hash on. Keeping the identifier in braces means any future companion keys for the same route can use the same braced part and land on the same shard. That is a reason to keep the braces and not simplify the pattern.

Rules for the key:

- Build it in one helper and nowhere else. No handler, worker or script should assemble the string by hand.
- Treat the route identifier as opaque. Do not parse meaning out of it.
- Do not put anything other than the route identifier into the key. Tenant, region and environment separation is handled by which Redis instance or logical namespace the service connects to, not by extra key segments.
- Scans over the whole keyspace are not allowed in request paths. If an operator needs a list of live routes, that comes from the planning store, not from walking Redis.

## What the hash holds

The hash is a flat map of fields to string values. Redis hashes do not nest, so anything structured is flattened or encoded before it is stored. The guiding rule is that each field should be something a reader might want on its own, so that a read of one field does not drag the rest of the route along.

General groups of fields in the hash:

- Position and progress: which leg the route is on, which legs are done, and whether the route is moving, waiting or held.
- Timing: planned and estimated times for upcoming handoffs, and the most recent delay estimate.
- Assignment: which vehicle, train or crew is bound to the active leg, and which are bound to the next.
- Load: a compact view of what is on board and what is waiting at the next transfer point.
- Bookkeeping: a version marker that increases with each accepted update, and a timestamp of the last accepted write.

Anything bulky, such as a full list of stops with every detail, should be stored as a single encoded field or kept out of the cache and fetched from the planning store when needed. The cache is for the hot summary. If a field is only ever needed by the solver, it does not belong here.

## Writers

There are only a few writers, and that is intentional. The more places that can change a route's live state, the harder it is to say why a value is what it is.

The main writer is the event consumer that listens to Google Cloud Pub/Sub. Telemetry, handoff confirmations, delay reports and dispatcher actions arrive as messages, and the consumer folds each one into the hash for the affected route. The second writer is the rebalancing path: when the solver produces a new assignment after a delay, the result is applied to the hash so that readers see the new assignment together with the reason it changed.

The FastAPI layer does not write route state directly on behalf of a browser or a client. A dispatcher action becomes an event, goes through the same consumer, and shows up in the hash like any other change. This keeps one path for state changes and makes replay easier to reason about.

Write rules:

- Update only the fields that changed, instead of rewriting the whole hash, so that concurrent updates to different fields do not clobber each other.
- Group related fields into one atomic step, either with a transaction or with a server-side script, so a reader never sees a new assignment with an old timing estimate.
- Compare the version marker before applying an update. A message older than the stored state is dropped, not applied.
- Never write a field with an empty placeholder to mean unknown. Remove the field, or leave it absent.

## Readers

Readers are the FastAPI endpoints that serve the dispatcher views, the rebalancer when it starts a run, and a few internal checks. Readers should ask for the fields they need and nothing more. A dashboard that shows many routes should batch its reads with a pipeline rather than issuing them one at a time.

A reader must treat a missing hash as a real answer. It can mean the route has not started, has finished and been cleaned up, or has expired after going quiet. The reader should not recreate the hash on a miss. If the caller needs more than the cache can say, it falls back to the planning store and reports the result as coming from there.

Readers should also be ready for a hash that is slightly behind the world. The cache reflects the last event the consumer applied, so a view built from it can lag by however long the event pipeline takes. Screens that show the state should show the last-updated time next to it, so a dispatcher can tell fresh from stale.

## Interaction with Pub/Sub

Pub/Sub delivers at least once and does not promise global ordering. The cache design has to live with that. Duplicates are handled by the version marker and by making each update idempotent: applying the same event twice leaves the hash the same as applying it once. Out-of-order delivery is handled by dropping anything older than what is stored, and by choosing event payloads that carry the full new value of a field instead of a difference to apply.

Ordering keys on the topic should use the route identifier, so that events for one route are delivered in order to the extent Pub/Sub can manage. That narrows the window for reordering but does not close it, so the version check stays.

When the consumer cannot apply a message, for example because the route's hash is missing and the event is not one that creates state, it should not drop the message silently. It should send it to a dead-letter path with enough context to retry once the cause is understood. A message that arrives before the route's initial state exists is the most common case, and it usually resolves by waiting for the creation event.

## Consistency and rebalancing

Rebalancing is the part that stresses the cache. When a delay hits, the rebalancer reads the live state of the affected routes, runs the solver, and writes new assignments back. Between the read and the write, new events may arrive. If the rebalancer blindly writes its result, it can overwrite fresher facts with decisions based on stale ones.

The rule is optimistic: the rebalancer remembers the version marker it read, and its write is accepted only if the marker is still the same. If it is not, the rebalancer re-reads and decides again, or hands the conflict to a dispatcher when the change is too large to apply automatically. This is slower in a storm of delays, but it is correct, and a dispatcher who sees an assignment change can trust that it was made against current information.

Across routes there is no cross-key transaction. A load that moves from one route to another is two writes to two hashes. The design accepts a brief window where both or neither show the load, and relies on the planning store as the reference for resolving it. Readers that care, such as a transfer-point view, should read both hashes and flag a mismatch instead of picking one silently.

## Expiry and cleanup

A hash for a finished route should not live forever. When a route completes, the consumer marks it complete and sets an expiry so that the data stays available for a short while for post-run views and then goes away. Routes that go quiet without a completion event are the harder case. They should also carry an expiry that is refreshed on each accepted write, so a route that has truly stopped receiving events ages out by itself.

The expiry should be generous compared with the longest normal gap between events on a route, otherwise a slow but healthy leg would lose its state mid-run. If a live route's hash does expire by mistake, the recovery path is to rebuild it from the planning store and then replay recent events, not to guess.

Redis memory is the constraint to watch. Because there is one hash per active route and each hash is kept small, memory should scale with the number of active routes. A sudden rise without a matching rise in routes points to hashes that are not being expired or to fields that have grown too large.

## Failure modes

The known ways this goes wrong, and what to do:

- Redis unavailable: reads fall back to the planning store with a clear indication that the view is not live. Writes from the consumer are not acknowledged, so Pub/Sub redelivers them. Because updates are idempotent, redelivery after recovery is safe.
- Consumer falls behind: the cache lags. Dispatcher views show an old last-updated time. Fix the consumer; do not patch the hash by hand unless a dispatcher needs an urgent correction, and then do it through the normal event path.
- Duplicate or reordered events: handled by the version check. If the stored version ever moves backwards, treat it as a bug in the writer.
- Partial writes: any code path that updates related fields without an atomic step is a bug, even if it has not shown up yet.
- Key drift: if some code builds a key that differs from `route:{route_id}:state`, its data is invisible to everyone else. Searching the code for hand-built keys is a cheap periodic check.
- Stale view after failover: if Redis fails over to a replica that missed recent writes, some routes will look slightly behind. The consumer's next event for each route corrects it, and the rebalancer's version check prevents it from acting on the gap.

## Open questions

Things not settled, to be decided before anyone builds on assumptions:

- Whether the hash should carry a compact change reason for the last assignment change, so dispatchers can see why the rebalancer moved a load without opening another system.
- Whether a clustered Redis is needed, which would make the braced part of the key matter in practice and not only as a precaution.
- How long finished routes should stay readable, which is a product question for dispatchers more than an engineering one.
- Whether the dead-letter path should retry on its own or wait for a human, for events that arrive before the route's state exists.
- Whether readers that span many routes should be served from a separate summary instead of many hash reads.

Until these are answered, keep the cache to what is written above: one hash per route at `route:{route_id}:state`, a small set of writers, idempotent versioned updates, and the planning store as the fallback and the source of truth.
