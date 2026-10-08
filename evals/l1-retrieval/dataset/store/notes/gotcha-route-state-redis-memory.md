---
id: 01KDZH5AK1MYD7W4GPZ04NY3NS
created: 2026-01-02T11:18-03:00
---

# route-state-cache write failures when Redis memory is full

When Redis memory is full, writes to the route-state-cache fail with `OOM command not allowed when used memory > 'maxmemory'`. Reads keep working. That is the trap: the cache looks healthy from the read side while every new route state is refused. Dispatchers then see stale legs, and the rebalancer works from state that is older than the delay it is meant to react to.

The short name is `rscache`. It is short for `route-state-cache`. You will see `rscache` in log prefixes, metric labels and chat, and `route-state-cache` in the service layout and the config. They are the same component.

## What actually fails

The error comes from Redis itself, not from our code. The server refuses any command that would grow memory once it is at its configured ceiling and the eviction policy does not allow it to free anything. The exact text is `OOM command not allowed when used memory > 'maxmemory'`. Our Python client raises it as a response error from the write call. It is not a connection error, so retry logic that only watches for dropped connections or timeouts will not catch it. Retrying a refused write immediately just repeats the refusal.

Only commands that add memory are refused. Deletes, expirations and reads still go through. That is why the cache can stay up and half-usable for a long time while it is full. Nobody gets paged by a crash, because there isn't one.

## Why the cache fills up

The route-state-cache holds the current state of every active multi-leg route: which leg each truck or rail consist is on, the planned and estimated times, the load assignments, and the last known delay. Each route has several entries, and the number of entries grows with the number of legs. Memory use therefore tracks the number of active routes times the legs per route, not just the number of routes.

Three things push it over the edge in practice:

- A burst of delays. When a rail disruption hits a region, the rebalancer rewrites state for many routes at once, and each rewrite can briefly hold both the old and new versions.
- Entries that never expire. If a writer forgets to set a time to live, finished routes stay forever. This builds up slowly and then looks sudden.
- Large values. Anyone who stuffs a full solver result into a cache entry, rather than a compact summary, multiplies the footprint.

None of these show up in a quiet test environment. The failure appears on a busy day, which is when the dispatchers most need fresh state.

## How it shows up in the planner

The OR-Tools solver does not talk to Redis directly. The planning worker reads the current state, builds the model, solves, and writes the new state back. When the write is refused, the solve itself has already succeeded. The result exists only in the worker's memory and is lost if the worker does not handle the error.

The bad outcome is a worker that logs the error and moves on. The next planning cycle then reads the old state, produces a plan against stale inputs, and may reverse or duplicate the earlier rebalance. Treat a refused write as a failed cycle, not as a logging event.

## How it shows up in the API

The FastAPI layer serves route state to dispatchers. Reads come from the route-state-cache, so they return normally, with old data. A dispatcher can see a route marked on time that has in fact been delayed for a while. Endpoints that accept dispatcher overrides also write to the cache, and those fail with a server error. If the error handler turns every unexpected exception into a generic failure response, the dispatcher sees only that something went wrong, with nothing pointing at memory.

When a bug report says that overrides do not stick but the map still loads, suspect this before suspecting the API code.

## How it shows up on the Pub/Sub side

Delay events arrive through Google Cloud Pub/Sub. The consumer applies each event to the cache and then acknowledges the message. The order matters. If the consumer acknowledges first and writes second, a refused write drops the event for good. If it writes first and acknowledges only on success, a refused write leaves the message unacknowledged, and Pub/Sub redelivers it later. That is the behavior we want, but it has a cost: while the cache stays full, the backlog grows and the redelivery rate climbs, which adds load right when the system is struggling.

Check the acknowledgement order in any new consumer. Also check that a refused write does not cause a tight redelivery loop. Back off instead of retrying at once.

## How to tell it is this problem

Start with the symptoms that appear together:

- Reads succeed and writes fail.
- The error text matches `OOM command not allowed when used memory > 'maxmemory'` in worker or API logs.
- Pub/Sub backlog for the delay subscription is growing while the consumers look busy.
- Route state timestamps in the dashboard stop advancing for some or all regions.

Then ask Redis directly for its memory report and compare used memory with the configured ceiling. Look at the eviction policy as well. If the policy is one that never evicts, a full instance will refuse writes. If it is one that evicts only keys with a time to live, a full instance of keys without one will refuse writes just the same. That second case is the one that has caught people, because eviction is enabled and still does nothing.

## What to do right now

When it is happening in production, the order is:

- Stop the bleeding by freeing memory that is safe to free. Finished routes and expired plans are the first candidates. Do not clear active route state.
- If that is not enough, raise the memory ceiling on the instance if the host has room. This is a stopgap and needs a follow-up.
- Once writes succeed again, make sure the Pub/Sub backlog drains and the planner re-runs for the affected regions. Cycles that failed during the outage have not been repaired by themselves.
- Tell dispatchers that state shown during the window may have been stale, and which regions were affected.

Do not restart the Redis instance as a first move. If persistence is not configured the way you assume, a restart loses the active route state, and rebuilding it from the planner is slow.

## Prevention

The durable fixes are mostly about discipline in writers:

- Every write to the route-state-cache sets a time to live. Finished routes should expire on their own, with a margin long enough for audits and short enough to matter.
- Keep values compact. Store what the dispatcher view and rebalancer need, and keep bulky solver output elsewhere.
- Pick an eviction policy on purpose. Evicting by least recent use among keys with a time to live is reasonable if every key has one. Document the choice next to the config.
- Alert on memory use well before the ceiling, not on the error. By the time the error appears, state is already stale.
- Make workers fail loudly on a refused write: surface the failure, do not acknowledge the message, and back off.

## Things that look like fixes but are not

A client-side retry with a short delay does not help, because the server stays full. Catching the error and falling back to reading from the cache does not help either, because that is exactly the stale state. Letting the planner skip the write and carry state in process memory trades a visible failure for a silent divergence between workers. Shrinking the number of regions served by one instance helps only if you also move their keys, and that is a planned change, not an incident response.

## Notes for whoever touches this next

If you add a new kind of entry to the route-state-cache, estimate its footprint per leg and say so in the change description. If you add a new writer, copy the time to live handling from an existing one rather than writing it fresh. If you see `rscache` in a metric name and the dashboard shows used memory near the ceiling, treat it as urgent even when no errors have shown up yet; the first refused write is usually only minutes behind.

Open question: whether a separate instance for finished-route history would be cheaper than tuning times to live on the shared one. Nobody has measured it. Until someone does, keep the shared instance and the discipline above.
