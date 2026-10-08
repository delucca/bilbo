---
id: 01JS8ASFQEE5PRMZXYM65W40FT
created: 2025-04-19T21:50-03:00
---

# leg-cost-matrix: common mistakes

Notes on how people trip over `leg-cost-matrix`. Nothing here is a specific failure, just the patterns that keep coming back. Read it before touching the matrix or anything that feeds it.

## Treating it as static

The matrix looks like a lookup table, so people build it once and keep it. It goes stale as soon as a delay arrives. Rebalancing needs costs that reflect the current state, not the plan from the morning.

## Mixing truck and rail units

Truck legs and rail legs often come from different sources with different units for time and cost. Adding them without normalising gives a matrix that solves fine and returns nonsense routes. Convert at the edge, once.

## Missing legs filled with zero

An absent leg is not a free leg. Filling gaps with zero makes OR-Tools love the gap. Use an explicit "not allowed" cost or drop the arc from the model.

## Integer scaling

OR-Tools wants integer arc costs. Rounding floats carelessly in different places produces ties that flip between runs. Pick one scale and one rounding point, and keep them together in one function.

## Index drift

Rows and columns are positional. If the node list is reordered or a stop is added, old cached matrices point at the wrong places. Always store the node ordering with the matrix and compare before reuse.

## Caching in Redis without a key that captures inputs

A cache key built only from the route id hides changes in delays or rates. Include whatever the costs depend on, or version the entries, and expire them.

## Rebuilding on every Pub/Sub message

Delay events can arrive in bursts. Recomputing the whole matrix per message wastes time and races with a solve in progress. Batch the events, and update only the affected legs where possible.

## Sharing a mutable matrix across requests

FastAPI handlers run concurrently. Mutating a shared matrix while a solve reads it gives half-updated costs. Hand each solve its own snapshot.

## Transfer and wait costs forgotten

Switching between truck and rail has handling and waiting cost that is not part of either leg. Leaving it out makes mode changes look cheap.

## Symmetry assumed

Cost from A to B is not cost from B to A, especially on rail with directional schedules. Do not mirror the upper triangle.

## Small sketch

A guard worth keeping before any solve:

```python
if matrix.nodes != model.nodes:
    raise ValueError("leg-cost-matrix is out of date for this model")
```

## When something looks wrong

Check the node ordering and the unit conversion first. Those two explain most odd routes before the solver is even suspected.
