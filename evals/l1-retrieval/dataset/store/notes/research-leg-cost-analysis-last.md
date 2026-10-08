---
id: 01KETDVP1T4NDKMCXRDGYCZ8XV
created: 2026-01-12T21:59-03:00
sources:
  - "doc: Q2 leg cost analysis"
---

# Leg-cost-matrix: rail dwell time and cost variance

This note records one finding about the cost model inside leg-cost-matrix, plus a naming point that trips people up when they read old code, old tickets or old dashboards. The finding came from an offline look at last quarter's legs, not from a live experiment, so treat it as a lead for tuning and not as a law of the system.

## Finding: rail dwell time drives leg cost

An analysis of last quarter's legs found that rail dwell time explains 38% of the variance in leg cost inside the leg-cost-matrix. That is the single biggest explanatory factor we looked at. Distance, truck hours, handling fees and the time-of-day surcharge all matter, but none of them came close to that share on its own.

What the figure means in plain terms: if you line up all the legs from last quarter and ask why some cost much more than others, about 38% of the spread is accounted for by how long loads sat waiting at rail points. It is a share of variance, not a share of total cost. A leg can have a large absolute cost with little dwell and still be explained mostly by other things. The number says nothing about causation by itself. Long dwell often goes along with congested terminals, missed connections and re-bookings, and any of those could be the real driver. We did not separate them.

Things to keep in mind when reading the result:

- The sample was last quarter only. Seasonal freight patterns, a bad weather stretch or a terminal outage in that window could inflate the dwell effect.
- The analysis looked at legs that were actually run. Legs the planner rejected never produced a dwell measurement, so they are absent. That biases the sample toward routes dispatchers already liked.
- Rail legs and truck legs share one matrix. Truck legs have no rail dwell, so the 38% is really a statement about the rail portion of the data and how it spreads the whole cost distribution.
- The figure came from a variance decomposition, so the factors overlap. Dwell correlates with delay events, and the share attributed to dwell may partly belong to delay.

## Naming: costgrid is the old name

costgrid was the previous name of leg-cost-matrix; the component is called leg-cost-matrix now. Both names refer to the same thing: the table of per-leg costs that the route planner reads when it builds multi-leg truck and rail routes and when it rebalances loads after a delay.

The old name still shows up in a few places, and it is easy to think they are separate components:

- Older commit messages, branch names and review comments talk about costgrid.
- Some Redis key prefixes and dashboard labels may still carry the old spelling. Do not assume a key with costgrid in it is stale or unused until you have checked what writes it.
- Earlier notes and tickets use costgrid, so a search for the new name alone will miss them. Search for both when you recall history.

In new text, code comments and notes, use leg-cost-matrix. Do not coin a third name. If you rename a leftover costgrid reference, do it as its own small change so it is easy to review and revert, and check that no consumer reads the old spelling before it goes.

## How this bears on the matrix design

The planner uses OR-Tools to choose among candidate routes, and the cost it minimizes comes straight from the matrix. If rail dwell is the largest source of spread, then the quality of the dwell estimate is the largest lever on how good the plans are. A matrix that treats dwell as a flat constant per terminal will be wrong in just the cases that matter most, which are the expensive legs.

Some consequences worth thinking through, none of them decided yet:

- Dwell as an input, not a fixed constant. The matrix could take a dwell estimate per rail leg that refreshes as new observations arrive, rather than a static value baked in at build time. Redis is the natural place to hold the latest estimate, since the matrix is already read from there at planning time.
- Rebalancing after delays. When a delay event arrives over Pub/Sub, the rebalance step needs fresh costs for the affected rail legs. If the dwell component is stale, the rebalance may pick a route that looked cheap an hour ago. The refresh path for dwell should be at least as fast as the path that triggers rebalancing.
- Sensitivity checks. Before changing anything, run the planner on a recent set of requests with the dwell term nudged up and down, and see how often the chosen route changes. If the plan barely moves, the variance finding matters less for routing than it does for cost reporting.
- Reporting versus planning. Variance in realized cost and variance in predicted cost are different things. This analysis used realized costs. Check how much of the realized dwell effect the matrix already predicts before assuming there is a gap to close.

## What we have not checked

A few gaps, so nobody reads more into the result than it supports.

We have not repeated the analysis on a different quarter, so we do not know whether the share is stable. If it swings a lot from quarter to quarter, a single number is a poor guide to tuning, and the right response is a model that tracks dwell continuously.

We have not split the result by region. Regional dispatchers use this system, and terminals differ a lot in how busy and how predictable they are. A strong overall dwell effect may be concentrated in a few terminals. If so, a targeted fix for those terminals is cheaper than a general change.

We have not tested whether dwell is a cause or a symptom. A missed connection produces both a long dwell and a re-booking charge, so the cost shows up beside dwell without dwell causing it. Separating these needs the delay and re-booking events joined to the legs, which the quarter's data may or may not support.

We have not looked at how the matrix is rebuilt. If rebuilds are batch jobs on a schedule, the dwell values it sees are only as fresh as the last run. That matters for the rebalance case above.

## Next steps

- Repeat the variance analysis on at least one more quarter and compare the dwell share against the figure here.
- Break the dwell effect down by terminal or region and see whether it is concentrated.
- Join delay and re-booking events to the legs and test whether dwell still carries weight after those are controlled for.
- Review how the matrix gets its dwell values today and how fast they update after a delay event.
- Sweep the planner's dwell term on recent requests to see how sensitive route choice is.
- When searching old material, use both leg-cost-matrix and costgrid.

If any of these changes the picture, update this note instead of starting a new one on the same subject.
