---
id: 01KCAKM10A0YQ950N2ZVEKW5KX
created: 2025-12-12T22:01-03:00
---

# route-planner load test: solve time on multi-leg plans

Load test of route-planner, written up quickly so the next session does not have to rerun it to learn the headline. In the test, route-planner solved plans of 40 stops across 3 legs with a median of 1.8 seconds per solve. That is the number to quote. Everything below is context for reading it, plus what I would check before trusting it for production sizing.

## What was measured

The test fed route-planner synthetic requests shaped like a regional dispatcher's day: a mix of truck and rail legs, with stops spread along the legs and time windows on most of them. Each request was one full plan from scratch, not a rebalance of an existing plan. A plan in the headline result had 40 stops across 3 legs. The figure is the median wall-clock time of a single solve, measured from the moment the planner received the request to the moment it had a plan ready to return.

The median is 1.8 seconds. I did not treat the median as the whole story. Solve time in a constraint solver has a long tail, and a median hides the slow cases that dispatchers notice. The tail is discussed below and I have no firm number for it in this note, so do not invent one from the median.

The solver is OR-Tools, driven from Python. The service around it is FastAPI. Requests arrive over HTTP, and the planner publishes results and listens for delay events through Google Cloud Pub/Sub. Redis holds cached data the planner reads while building a plan. The test exercised the solve path mainly; the Pub/Sub round trip was not part of the 1.8 seconds.

## What is inside the solve time

The 1.8 seconds covers building the routing model from the request, running the OR-Tools search, and turning the solution back into the response structure. It does not include network time to the caller, and it does not include any wait before the request reached a worker.

Model building is not free. For a plan of 40 stops across 3 legs, a noticeable share of the time went into assembling the transit data and constraints rather than into search. That share would matter if someone tries to speed things up by tuning search parameters alone; the model construction would stay the same. I did not profile this finely enough to give a split, so treat it as a lead, not a finding.

The search itself runs under a time limit. If the limit is the thing that ends most solves, then the median is partly a reflection of the limit rather than of how hard the problem is. Check how the limit is set before comparing numbers between runs. A median near the limit would mean the solver is being cut off, and plan quality, not speed, is then the thing to look at.

## Caveats on the number

The load test used synthetic data. Real routes have messier time windows, uneven stop density, and rail legs with fixed schedules that make some combinations infeasible. Infeasible or nearly infeasible requests can take far longer than the median or fail outright, and the synthetic set had few of them.

The test ran on one machine class and one worker setup. Concurrency changes solve time, since solves compete for CPU. The headline is a per-solve median, not a throughput figure, and it should not be multiplied out to claim how many plans per minute the service handles.

Plan size matters a lot. The result applies to plans of 40 stops across 3 legs. Smaller plans solved faster and larger ones slower, but I am not putting numbers on those here. Do not extrapolate the 1.8 seconds to plans that are much bigger; solver time tends to grow faster than linearly with stops.

Cache state in Redis affects the result. The run was done with the cache warm. A cold cache adds reads and would push the median up. If a later run shows a worse median, check cache warmth before blaming the solver.

## Why it matters for rebalancing

The point of route-planner is not only first plans. When a delay arrives through Pub/Sub, the planner has to rebalance loads, and dispatchers expect that to feel quick. A full solve at a median of 1.8 seconds is acceptable for initial planning. For rebalancing, a full from-scratch solve is the worst case; a rebalance that keeps most of the existing plan fixed and only reworks the affected legs should be faster. That is a design hypothesis, not something this test confirmed.

If several delays arrive close together, solves could queue behind each other, and then the wait before a solve starts would dominate what the dispatcher sees, not the 1.8 seconds. That queueing was not measured.

## Follow-ups

Things worth doing next, roughly in order of value:

- Record the tail of the distribution, not only the median, on the same plan shape, so there is a number for the slow solves.
- Rerun with a cold Redis cache and compare against the warm result.
- Test with real, anonymized dispatcher requests, including infeasible ones, and see how the solver time limit behaves.
- Profile model construction separately from search, to know how much of the median is avoidable without touching the solver.
- Measure a rebalance triggered by a delay event end to end, including the Pub/Sub hop and any queueing.
- Run several solves at once to see how the median moves under concurrency.

Until those are done, the safe statement is narrow: on this synthetic test, route-planner solved plans of 40 stops across 3 legs with a median of 1.8 seconds per solve, warm cache, one machine class, solves run on their own.
