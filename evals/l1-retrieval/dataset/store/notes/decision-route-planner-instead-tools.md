---
id: 01KDG2R84QZWM8X74RMVP5PDDF
created: 2025-12-27T11:17-03:00
sources:
  - "doc: Planner solver selection memo"
---

# route-planner solver choice: CP-SAT instead of RoutingModel

The route-planner uses CP-SAT instead of the OR-Tools `RoutingModel` because rail timetables with fixed departures and transfer dwell times are hard to express as routing dimensions. That is the decision. Everything below is the reasoning, what it costs us, and what to watch for. The internal codename of route-planner is `lanehound`, so you will see that name in older branches, dashboards, deployment labels and chat history. It is the same component as `route-planner`, not a separate service.

## Decision

route-planner builds its multi-leg truck and rail plans as a CP-SAT model from OR-Tools. It does not use the OR-Tools `RoutingModel` and its routing library. We looked at `RoutingModel` first, since a freight network looks like a vehicle routing problem at a glance. We dropped it because rail does not fit the way it wants to see time.

In short: the route-planner uses CP-SAT instead of the OR-Tools `RoutingModel`, because rail timetables with fixed departures and transfer dwell times are hard to express as routing dimensions. Trucks alone would have fit `RoutingModel` fine. Rail is what broke it, and the product needs both modes in one plan.

This note covers the solver choice only. It does not cover the API layer, the cache layout or the event wiring, except where they touch the solver.

## Why RoutingModel did not fit

In `RoutingModel`, time is a dimension: a cumulative quantity that accumulates along a vehicle's path, with slack and optional windows at each node. That is a good fit when a vehicle can leave a stop whenever it is ready, within a window. A truck is like that. A train is not.

Three things went wrong when we tried to model rail that way.

- Fixed departures. A train leaves at a published time. It is not a window we can slide into; it is a point, or a small set of points across the operating pattern. In a dimension you can fake this with tight windows and zero slack, but then every other node on the same vehicle path inherits that rigidity, and the solver spends its effort proving infeasibility around the fake constraint instead of searching.
- Transfer dwell times. When a load moves from a truck to a train, or between trains, there is a minimum dwell for handling, and it depends on the terminal and on the pair of modes. That is a constraint between two different legs belonging to different vehicles. Dimensions are per vehicle. A transfer is a coupling between vehicles at a shared place and time, and the routing library has no natural way to say it. We ended up with auxiliary nodes and extra dimensions whose only job was to carry the coupling, and the model became hard to read and hard to debug.
- Shipments on more than one leg. A single load can ride several legs on different carriers. In the routing library each shipment is a pickup and delivery pair on one vehicle. Splitting a shipment across vehicles needs transshipment nodes that we had to invent and then keep consistent with the real terminals. Every invented node was another place for the plan to disagree with what a dispatcher sees on the board.

None of these is impossible in `RoutingModel`. The point is that each one needed a workaround, the workarounds interacted, and a dispatcher asking why the planner chose a given transfer could not be answered from the model without reading the workarounds. With CP-SAT the same facts are direct constraints and we can say them in the same words a dispatcher uses.

## What the CP-SAT model looks like

The model is explicit. We state what we mean instead of encoding it into a library's vocabulary.

- Each candidate leg is a decision about whether a load uses it. Truck legs and rail legs are the same kind of object in the model, with different timing rules.
- Interval variables carry the time of each leg. A rail leg has its start fixed to a timetable departure, chosen from the departures that exist for that service. A truck leg has a flexible start with a duration from the travel estimate.
- A transfer is a constraint between the end of one leg and the start of the next, requiring at least the terminal's dwell time. This is the constraint that was awkward before. Here it is a single inequality between two time expressions, guarded by whether both legs are chosen.
- Capacity on trucks and on rail cars is a cumulative-style constraint over the legs that share a vehicle or a train.
- The objective combines cost, lateness against the promised delivery, and a penalty for plans that are fragile, meaning that a small delay would break a transfer. The weights are configuration, not constants in the code, and dispatchers' feedback decides how they move.

Because the structure is explicit, a failed solve can be explained. When the model is infeasible we can say which transfer or which departure made it so, and the API can return that reason instead of a bare failure. That is a practical gain over the routing library, which tended to give back no plan and little else.

## Rebalancing when delays occur

The other half of the product is rebalancing loads after delays. The same model supports it, which was a second reason to prefer CP-SAT.

When a delay event arrives, the planner does not start from nothing. It rebuilds the model for the affected loads with the new times as facts: a train that is late is either held to a later departure that exists, or the load is treated as having missed it and must use another option. Legs already under way are fixed. Legs not yet started are open. CP-SAT lets us add the previous plan as a hint, so the new solve begins close to what dispatchers already know and changes as little as it can. Dispatchers dislike plans that move more than necessary, so a minimal-change term is part of the objective in rebalancing.

With the routing library this kind of fix-and-resolve was possible but clumsy, since changing the fixed part of a path meant rebuilding the whole routing structure. In CP-SAT, fixing a leg is one more constraint.

## How it fits with the rest of the system

route-planner sits behind a FastAPI service. Dispatch requests come in over HTTP and delay events arrive from Google Cloud Pub/Sub. Redis holds the working state that must be shared between requests and workers, such as the last plan for a load and the cached timetable data the model needs.

The solver choice has a few consequences for these pieces.

- Solves are CPU-bound and can take a noticeable time. They must not run on the event loop of the web process. The API hands the work to a worker and answers with a plan or a handle, depending on the request. Do not call the solver directly from an async handler.
- Every solve has a time limit. A good but not proven plan within the limit is acceptable, and the response says whether it was proven optimal. Dispatchers need an answer during an incident more than they need a proof.
- Pub/Sub delivers at least once, so the same delay event can arrive twice. Rebalancing is idempotent for a given event: the second delivery produces the same decision or is skipped. The plan stored in Redis records which event it already accounts for.
- The timetable data used by the model is read from Redis at the start of a solve. If it changes mid-solve, the solve does not see the change. The next delay event picks it up.

## Costs we accepted

Choosing CP-SAT is not free, and the team should not forget what we gave up.

- We lose the routing library's built-in search strategies and local search tuned for vehicle routing. For large truck-only instances the routing library is likely faster than a general model. We accept that because our instances mix modes and are bounded by regional operations, not by continental networks.
- We maintain more model code ourselves. The constraints are readable, but there are more of them to keep right, and each one needs a test. A bug in a transfer constraint will produce plausible but wrong plans, which is worse than a crash.
- Performance depends on how we formulate things. Adding many optional legs that are never useful makes the model larger without making it better. Candidate generation, meaning which legs we offer to the solver, matters as much as the solver settings. Prune candidates before the model, not inside it.
- The knowledge of the routing library in the team, and in the wider community, does not transfer. New people need to learn interval variables and optional intervals. A short walkthrough in the repository docs helps.

## Gotchas

- Time units. The model works in integers. Convert from timestamps once, at the boundary, using a single granularity, and convert back once. Mixing granularities between truck estimates and timetable data produces off-by-granularity transfers that look like tiny violations.
- Timetable departures that repeat on a pattern must be expanded into concrete departures inside the planning horizon before the model is built. Do not try to express the pattern inside the model.
- A load with no feasible option should be reported to the dispatcher as unplaceable, with the reason, not silently dropped from the plan.
- Determinism. With multiple search workers, the same input can give different, equally good plans. For tests, fix the seed and use a single worker, or assert on the properties of a plan (feasible, within cost bound) instead of its exact shape.
- Old docs and some metric names say `lanehound`. Searching for it finds the history of route-planner. When you add new names, use `route-planner`.

## When to revisit

Revisit this decision if one of these becomes true.

- Rail stops being a first-class part of the product, or becomes so regular that a simple recurring pattern replaces explicit timetables. Then a pure truck problem would fit `RoutingModel` well, and its search could beat our model.
- Solve times for the largest realistic requests stop fitting inside the time limit even with pruning, and a decomposition by region does not help.
- The routing library gains a clean way to express coupled transfers between vehicles with fixed departures. We have not seen it, but we would check before building anything new.

Until then, the answer to why it is not `RoutingModel` is the one at the top: rail timetables with fixed departures and transfer dwell times are hard to express as routing dimensions, and CP-SAT states them directly.
