---
id: 01JXK0G91Y6NV2ZVNNEZNHYTV1
created: 2025-06-12T18:24-03:00
---

# route-state-cache: general direction

We settled on a general direction for route-state-cache and this note keeps the shape of it, so nobody has to rebuild the argument later. It has no tuned settings on purpose. Those live elsewhere and change often. What is here is the direction and the reasons.

route-state-cache holds the current working picture of each planned route: which legs exist, which vehicle or train slot each leg rides on, and where each load sits right now. The planner and the rebalancer both read it all the time. Dispatchers see it through the API. It sits in Redis, in front of the slower planning work done with OR-Tools.

## What we chose

The cache is a read-optimized copy of route state, not the place where truth lives. The planner output and the event history remain the record. If the cache is wiped, we can rebuild it. We accept that a rebuild is slow and a little ugly, because it keeps the cache simple and keeps us from treating Redis as a database.

Writers go through one narrow path. Nothing else touches route keys directly. That path is the only code that knows the key layout, so a layout change is a change in one place.

We prefer replacing a route's state as a whole over patching single fields in place. Whole-record replacement is easier to reason about when two delay events land close together. Partial patches were tempting because they are cheaper, but they gave us half-updated routes that looked valid and were not.

## Why

Dispatchers care more about a route that is slightly stale and internally consistent than one that is fresh in one leg and old in the next. A consistent view lets them make a call. A mixed view makes them distrust the screen, and then they phone each other, which defeats the product.

Rebalancing after a delay touches many routes at once. If the cache allowed partial writes, a rebalance in progress would be visible half-done. So the direction favors updates that appear all at once from the reader's side.

We also did not want the cache to become a second planner. Logic about which leg is feasible belongs in the planning code. The cache stores results and does not decide anything.

## Freshness and invalidation

Freshness is driven by events, not by guessing. Delay and status messages arrive over Google Cloud Pub/Sub, and a consumer applies them to the cache. Time-based expiry exists only as a safety net for abandoned routes, not as the main way state gets refreshed. We do not want a route quietly disappearing from the cache during a live shift because a timer ran out.

Messages can arrive twice or out of order. The consumer has to tolerate both. When it sees something older than what is stored, it drops it. When it sees a repeat, applying it again must change nothing. We lean on this instead of trying to get exactly-once delivery, which Pub/Sub does not give us for free.

If the consumer falls behind or loses messages, the fallback is to rebuild the affected routes from the record instead of trying to repair them by hand.

## What readers can rely on

Readers can rely on a route being either the old complete version or the new complete version. They cannot rely on the cache being the latest version at every instant. Anything that needs the latest answer, such as committing a rebalance, must check against the record and not trust the cache alone.

API handlers read from the cache and fall back to the record on a miss. A miss should be rare and should not be treated as an error. The shape of what handlers return is described in [[dispatch-api-request-models]], and the cache should not leak its own storage format into those responses.

## Things we decided against

- Using the cache as the only copy of route state. Too risky for something dispatchers act on.
- Letting each service build its own keys. We tried the idea on paper and it drifts quickly.
- Fine-grained field updates for speed. Consistency mattered more than the saved work.
- Relying on expiry for correctness. It hides bugs and makes live routes vanish.
- Locking readers during a rebalance. It would stall dispatchers at the worst moment.

## Open questions

How much history, if any, should the cache keep per route for the "what changed" view dispatchers keep asking for. For now the answer is none, and that view is served from the record.

Whether very large multi-leg routes should be split into pieces. Splitting would make writes smaller, but it would reopen the half-updated problem. Not decided, and we should not split until we have a way to keep the pieces consistent.

How to warm the cache after a restart without hammering the record. The simple approach works today. If it stops working, revisit it, but keep the rule that the cache can always be rebuilt.

## If you change this

Check the three assumptions first: the record stays authoritative, writes go through the one path, and updates replace whole routes. If a change breaks any of them, write a new decision note and say why, instead of editing this one quietly.
