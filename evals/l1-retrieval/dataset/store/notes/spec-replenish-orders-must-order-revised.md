---
id: 01M2VV5RC6Q9PJYBZZ8PCDVSKE
created: 2026-09-19T00:25-03:00
---

# replenish-orders-job spec: order line cap

This note replaces the earlier note "replenish orders must order"; the new value is that the replenish-orders-job order line cap is now 500 units, replacing the earlier cap, after analysts asked for larger pallet orders.

To be plain about it: the cap applies to a single order line, meaning one product at one store on one generated replenishment order. The value is `500 units`. Merchandising analysts at the grocery chains asked for this because the old cap forced pallet-sized quantities to be split or trimmed. The job's other behaviour is described below and did not change with this edit. If you are reading this to answer "what is the cap", the answer is `500 units` per order line.

## What the job does

replenish-orders-job is the part of ShelfSense that turns stockout predictions into replenishment orders. The prediction side scores each store and product pair for the risk of running out. This job takes those scores, together with current stock position, expected deliveries and the ordering rules a chain has configured, and produces a set of proposed order lines. Analysts review those lines and release them to the chain's ordering system.

The job is written in Scala and runs on Apache Spark. It reads its inputs from Delta Lake tables, writes its outputs to Delta Lake tables, and a later step publishes the final order lines to Snowflake, where analysts and downstream reports read them. Airflow schedules and orders the runs. The job itself holds no state between runs beyond what is in the tables.

The job is meant to be deterministic for a given set of inputs. Running it again over the same input snapshot should give the same order lines. That matters for the cap, because the cap is applied as a final adjustment and must not make results depend on partition layout or row order.

## The cap rule

The rule is simple. For each proposed order line, the quantity ordered is the smaller of the quantity the model and the ordering rules call for, and the cap. The cap is `500 units`. Quantities at or below the cap pass through unchanged. Quantities above the cap are reduced to the cap.

The cap is per line, not per order and not per store. An order can contain many lines, each of which may sit at the cap. Nothing in the job limits the total across lines because of this rule. If an analyst wants a total limit, that is a separate rule and not part of this spec.

The cap is measured in units of the product as it is ordered, which is the same unit the stock position uses. It is not measured in cases or pallets. The pallet reference in the request is the reason the number went up, not a unit of measure in the code. Someone converting a pallet quantity to units must do that before comparing against the cap.

```
order_line_cap = 500 units
replenish-orders-job: qty = min(requested_qty, order_line_cap)
```

The block above is shorthand for the rule, not the literal configuration syntax.

## Why the cap changed

The earlier cap was lower. Analysts reported that for fast-moving products with pallet-based supply, the old cap cut orders below what the supplier would ship as a full pallet. That left stores short, or pushed analysts to edit lines by hand after the job ran. Both are the opposite of what the job is for.

The decision was to raise the cap to `500 units` and keep it as a single global value rather than make it vary by product. The reasoning was that a single value is easy to explain and check, and that the analysts' complaint was about the size of the ceiling, not about the lack of per-product control. If that turns out wrong, a per-product ceiling would be a new spec.

This note does not restate the old value. Anyone who needs it can look at the history of the earlier note. The only value that should be used from now on is `500 units`.

## Inputs

The job reads several groups of inputs. First, the stockout risk scores from the prediction side, per store and product. Second, the current stock position, as last loaded from the stores. Third, open orders and expected deliveries, so the job does not order what is already on its way. Fourth, the ordering rules and parameters for each chain, such as pack sizes and minimum order quantities. Fifth, the cap.

All of these are read from Delta Lake tables at a consistent snapshot, so a run sees one coherent view. The job should record which snapshot of each table it used, so a result can be traced and reproduced later.

If an input is missing or clearly stale, the job fails the run rather than ordering from partial data. A failed run is better than a wrong order, because analysts review the output and a silent error is harder to catch than a visible one.

## Outputs

The main output is a table of proposed order lines. Each line carries the store, the product, the quantity after the cap, the quantity before the cap, and a flag showing whether the cap changed it. Keeping the quantity before the cap is deliberate. It lets analysts see how often the cap bites and lets us judge whether `500 units` is a good value.

A second output is a small run summary: how many lines were produced, how many were capped, and which snapshots were used. Airflow can read this to decide whether a run looks reasonable.

The final step publishes the lines to Snowflake. The publish should be idempotent, so a retried task does not duplicate lines. The usual approach is to replace the lines for a run as a unit instead of appending.

## How the cap is applied in the code

The cap is applied late. The job first works out the requested quantity from the risk score, stock, deliveries and rules. Then it rounds to the pack size. Then it applies the cap. Order matters here, and we should not move the cap earlier.

If the cap were applied before rounding to pack size, rounding up could push a line back over the cap. If the cap were applied before the minimum order rule, the minimum could override it. The agreed behaviour is that the cap is the last word on quantity: nothing after it may raise a line above `500 units`.

A consequence is that a capped quantity may not be a multiple of the pack size. That is accepted for now. If the chain's ordering system rejects such lines, the fix is to round the capped quantity down to a pack multiple, and that would be a change to this spec. Until someone reports it as a problem, the job does not do this.

In Scala the cap is a plain value read from configuration and passed into the transformation, not a literal inside the transformation. That keeps the transformation testable with different caps and keeps the number in one place.

## Configuration

The cap lives in the job's configuration, not in the code. The configuration value is set to `500 units` for production. Changing it should go through normal review, because it changes what stores get ordered.

The job should refuse to start if the cap is missing, zero or negative. A missing cap should never mean no cap. Falling back silently to unlimited would be the worst failure here, since it could produce very large orders that analysts only notice later.

Other environments, such as test and staging, may use a different value so tests can show the cap biting with small data. Production must use the value in this note. If you find production set to anything else, treat that as a bug and check this note first.

## Scheduling and Airflow

Airflow runs replenish-orders-job after the prediction side has finished and before the publish to Snowflake is considered complete. The dependency is explicit in the DAG, so the job does not start on stale scores.

Retries are allowed because the job is deterministic and the publish is idempotent. A retry after a partial failure should give the same result as a clean run. If a retry ever gives different capped quantities, something is reading a moving input and that needs fixing.

The cap change does not alter the schedule. It does not need a backfill of past runs. Orders already released stay as they were. Only runs from the change onward use the new cap.

## Testing

There should be tests for the cap at the edges. A quantity below the cap stays unchanged. A quantity exactly at the cap stays unchanged. A quantity above the cap is reduced to the cap. A very large quantity is reduced to the cap and not overflowed or wrapped. Each test should also check the flag that marks a line as capped, and the stored pre-cap quantity.

There should also be a test that the cap is applied after pack rounding and after minimum order handling, since that order is part of the rule. A test where rounding up would exceed the cap catches anyone who moves the cap earlier.

At the job level, a small end-to-end test over a handful of stores should confirm that the run summary counts of capped lines match the flags in the output table.

## Operational notes

When analysts ask why a line is smaller than they expected, check the capped flag first. If it is set, the model wanted more than `500 units` and the cap held it back. That is working as intended, and the pre-cap quantity shows what was wanted.

If analysts say they still see too many capped lines, collect the pre-cap quantities for those lines before changing anything. The number of capped lines tells us whether the value is right. A handful of extreme lines is a model question, not a cap question.

If the ordering system rejects a line, look at whether it was capped and whether the capped quantity breaks a pack rule. That is the known soft spot described above.

## Open questions

Whether the cap should vary by product or by chain remains open. Right now one global value is enough and nobody has asked for more. Whether capped quantities should be rounded down to a pack multiple is also open, and depends on what the ordering systems accept. Whether a total-per-order limit is needed is a separate question and would get its own spec.

Until one of these is raised with evidence, the rule stands as written: each order line is limited to `500 units`, the cap is applied last, and it replaces the earlier cap that this note's predecessor described.
