---
id: 01KK42VSXHF7C5NTEMMZTDXKYY
created: 2026-03-07T09:02-03:00
---

# Leg cost analysis, second pass

Notes from poking at how `leg-cost-matrix` gets built and what it costs us at rebalance time. This is partial. I did not go through everything, and some of it is from memory of reading the code, so check before relying on it.

## What the matrix is

`leg-cost-matrix` holds the cost of moving a load across each leg, truck or rail, between pairs of terminals. The OR-Tools model reads it as its arc cost. Each row is an origin, each column a destination, and the cell is a single combined number that already folds in distance, handling and a delay penalty. That folding is the part I keep tripping over: you cannot tell from the cell alone why a leg is expensive.

## Where it lives

The built matrix is cached in Redis so the FastAPI handlers do not rebuild it on every request. The key is per planning region. The cache lifetime is the usual one we set for planning data, which is short enough that stale costs are not a big worry in normal operation, but long enough that a burst of delay events can outlive it in the wrong direction. I have not measured how often that happens.

## Rebuild triggers

Delay events arrive over Pub/Sub. A consumer marks the affected legs dirty and the matrix gets patched, not fully rebuilt. Full rebuilds happen on a schedule and when the cache is cold. The patch path is where I would look first if costs ever look wrong, because it only touches cells for the legs named in the event. A delay on one leg that should also change the cost of a connecting leg (for example a rail leg that feeds a truck leg) does not propagate unless the event names both.

## Observations

- Truck legs are noisy. Rail legs are stable until a delay hits, then jump.
- The delay penalty dominates for short legs. For long legs distance dominates and the penalty hardly shows.
- Asymmetric pairs exist. Going A to B is not the same cost as B to A, mostly because of terminal handling, so never assume the matrix is symmetric when debugging.
- Some cells are effectively blocked. They hold a large sentinel cost instead of being missing, which keeps the solver from crashing but means a blocked leg can still show up in a solution if there is no alternative.

## Open questions

- Should the penalty be kept as its own matrix and added at solve time? It would make the cells explainable and the patch path simpler.
- Is the sentinel for blocked legs large enough relative to real costs on the longest routes? I think so but did not verify.
- How much of the rebalance latency is matrix patching versus the solver itself? I suspect the solver, but I have no timing to show it.

## Quick check I used

Reading one row back from the cache to compare against what the solver was given:

```python
# redis client from the app config; region key as used by the planner
row = redis_client.get(region_key)
```

That only confirms the cache matches the solver input. It says nothing about whether the cost itself is right.

## Next steps

1. Write down how the combined cell is computed, in one place, since right now it is spread over the builder and the patch consumer.
2. Add a test where a delay on a rail leg should change a connected truck leg, and see whether it fails.
3. Time the patch path against the full rebuild with a realistic region and the usual event rate.
4. Decide on splitting out the penalty matrix after those results.
