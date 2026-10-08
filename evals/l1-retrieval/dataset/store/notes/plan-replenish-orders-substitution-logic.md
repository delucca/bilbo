---
id: 01KYW3ZN8V8AN7V0TRJ71969B7
created: 2026-07-31T09:56-03:00
---

# replenish-orders-job: substitution logic plan

The replenish-orders-job will get substitution logic for SKUs whose `lead_time_days` exceeds 5, proposing a sibling SKU instead of a late order. That is the whole change in one line: when a SKU's `lead_time_days` is greater than 5, the job stops emitting an order that would arrive late and emits a proposal for a sibling SKU instead. SKUs with `lead_time_days` of 5 or less keep the current behaviour and get a normal order. This note is the plan for it, written quickly, so some details are still open and are marked as such.

Naming: `reorderer` was the previous name of this component. It is called `replenish-orders-job` now. Old Airflow DAG ids, dashboards, Snowflake comments and some Slack threads may still say `reorderer`. Treat them as the same thing. When searching the repo or the docs for history, search for both names. New code, config and docs should only use `replenish-orders-job`.

## Why

Merchandising analysts at the grocery chains use the orders the job generates. When a supplier takes a long time to deliver, the stockout the prediction flagged will happen anyway before the order lands. The order is then correct on paper and useless on the shelf. Analysts have been fixing these by hand: they look at the late order, find a similar product, and swap it. The goal is to do that first pass automatically and leave the final decision to the analyst.

## What the change does

- Read `lead_time_days` per SKU from the same source the job already uses for supplier data. No new source is needed.
- For each SKU where `lead_time_days` exceeds 5, do not write a normal replenishment order line.
- Look for a sibling SKU, meaning a product in the same category and similar size or brand tier, that is stocked at the store and has a shorter lead time.
- If a sibling is found, write a substitution proposal that names the original SKU, the sibling SKU, and the reason (lead time too long).
- If no sibling is found, fall back to the normal order and flag it as late, so the analyst still sees the stockout risk. Dropping the order silently would be worse than a late one.

The threshold of 5 should live in job config, not be hard-coded in the Scala. The value is 5 for now. Changing it later should not need a release.

## Open questions

- How to define sibling. The simplest rule is same category plus same pack-size band. Better rules may use sales correlation or past substitution behaviour. Start with the simple rule and keep the matching in one function so it can be replaced.
- Whether a proposal consumes the sibling's own forecast. If many stores swap to the same sibling, its demand rises and its own stockout risk goes up. First version: ignore this, but log how often it happens so we can judge the size of the problem.
- Whether proposals go into the same Delta table as orders or into a separate one. Leaning towards a separate table with a join key back to the order run, so existing consumers of the orders table do not break.
- How analysts approve or reject a proposal. Probably a column in the Snowflake-facing view. Check with the analysts before building anything for it.

## Implementation steps

1. Add the config key for the lead time threshold and read it in the job setup.
2. Add the sibling lookup as a separate, unit-testable Scala function that takes the SKU, the store assortment and the supplier lead times.
3. Branch in the order generation: SKUs over the threshold go to the substitution path, the rest go through unchanged.
4. Write proposals to the new Delta table, and make sure the schema change is additive so existing readers are not affected.
5. Expose the proposals to Snowflake the same way the orders are exposed today.
6. Update the Airflow DAG only if the new table needs its own task or sensor. Prefer to keep it inside the existing job run.
7. Add tests: a SKU just under the threshold, a SKU just over it, a SKU over it with no sibling, and a SKU with missing `lead_time_days`.

## Risks and gotchas

Missing or null `lead_time_days` must not be treated as over the threshold. Treat null as unknown and keep the normal order path, then log a count of those SKUs. Otherwise a bad supplier feed would turn most orders into proposals.

A sibling must itself be orderable at that store. A sibling that is delisted or not in the store's assortment makes the proposal worse than the late order.

Backfills and reruns of the job should give the same proposals for the same inputs. Keep the sibling choice deterministic, with a stable tie-break such as SKU id order.

## Related

Upstream data timing affects this: if sales data arrives late, the stockout predictions that drive orders are stale, and the substitution path inherits that. See [[sales-ingest-next-steps]] for the ingest side.

## Status

Not started. Nothing is merged. The next action is to confirm the sibling rule with the analysts, then do steps 1 to 3 behind a config flag that defaults to off.
