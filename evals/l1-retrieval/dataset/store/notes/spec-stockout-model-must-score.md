---
id: 01KSEDDK98Y44V8VR4GZE75TC8
created: 2026-05-24T22:54-03:00
sources:
  - "doc: ShelfSense Nightly SLA"
---

# stockout-model spec

This note holds the spec for stockout-model, the part of ShelfSense that predicts store-level stockouts. Its output feeds the replenishment orders that merchandising analysts at grocery chains review. The internal codename is oosnet, so expect to see that name in logs, branches and chat. Both names mean the same component.

## Purpose

stockout-model estimates, for each store and product, how likely a shelf is to run out before the next delivery. Replenishment order generation reads those scores. Without fresh scores the orders are built on stale data, and analysts notice.

## Codename

oosnet is the internal codename of stockout-model. Use `stockout-model` in docs and notes. Searching for either name should find the same thing. If you see oosnet in a job name or a config key, it is this component and not a separate service.

## Timing requirement

The stockout-model must score every store within 45 minutes of the nightly sales load finishing. The clock starts when the nightly sales load is done, not when the scoring job is scheduled. The clock stops when the last store has its scores written. A run that scores most stores in time but leaves some late still misses the requirement.

## Why the deadline exists

Analysts start their day by looking at proposed orders. Orders must be ready before deliveries are cut off for the morning. The 45 minutes is the budget that leaves room for order generation after scoring.

## Scope of "every store"

The requirement covers all stores of all chains that load sales that night. It is not an average or a percentile. A single slow store counts against the requirement, so skewed stores matter more than the mean runtime.

## Inputs

The model reads the sales data loaded overnight, plus history and store attributes already stored in Delta Lake tables. It should not depend on anything that arrives after the nightly sales load, or the timing budget stops being meaningful.

## Outputs

The model writes per-store, per-product stockout scores. Downstream, replenishment order generation consumes them, and Snowflake serves the analyst-facing views. Keep the score schema stable, since consumers read it directly.

## Where it runs

Scoring is a Scala job on Apache Spark. Airflow triggers it after the nightly sales load task succeeds. Delta Lake holds the inputs and the written scores.

## Trigger

The Airflow dependency should be on the completion of the sales load, so scoring starts right away. Avoid fixed start times with a safety gap, since that gap eats the 45 minutes or hides late loads.

## Code sketch of the dependency

```
sales_load >> stockout_model_score >> replenishment_orders
```

## Performance levers

When the budget is at risk, look at these in order: partitioning of the input by store, skew from very large stores, cluster size for the nightly window, and the cost of feature preparation compared with the scoring itself.

## Skew

Big stores carry far more product rows than small ones. If partitions follow store, a few tasks can run long and decide the finish time. Check task duration spread before adding machines.

## Failure handling

If scoring fails partway, a rerun should finish only the stores not yet scored, if possible, so a retry does not spend the budget twice. Writes must be safe to repeat.

## Monitoring

Track the time from sales load finish to last store scored on every run, and alert when it nears the limit, not only when it is exceeded. Keep this measure per run so trends show up before a miss.

## Open questions

Whether late-arriving corrections to sales data should trigger a partial rescore is not settled. Nor is what happens when the sales load itself finishes unusually late.

## Related work

Order generation has its own timing needs that depend on this one. Any change that makes scoring slower should be raised with whoever owns replenishment.

## Summary of facts

- Component: stockout-model, codename oosnet.
- Requirement: score every store within 45 minutes of the nightly sales load finishing.
