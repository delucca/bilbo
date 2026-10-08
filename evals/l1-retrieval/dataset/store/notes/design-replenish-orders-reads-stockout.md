---
id: 01JSSQDG1PC76KX43HPBB7P13W
created: 2025-04-26T15:58-03:00
---

# replenish-orders-job design

This note records how replenish-orders-job is shaped and why. It is written from what the team knows about the job as a design, not from a fresh read of the code, so check details against the repo before relying on them. The job is the last stage of the ShelfSense chain. Upstream models estimate, per store and per item, how likely a stockout is. This job takes those probabilities and turns them into proposed replenishment orders that merchandising analysts at grocery chains can review.

The core fact: replenish-orders-job reads stockout probabilities and writes proposed orders to the Delta table `replenish_orders`, partitioned by `order_date`. Everything else in this note is context around that read and that write. If you remember only one thing, remember that the output is a partitioned Delta table, that the partition key is the order date, and that the job proposes orders. It does not place them.

## Purpose and scope

The job answers one question each run: given what we now believe about the chance of each item running out in each store, what should we propose to order, and how much? It is written in Scala on Apache Spark. It is scheduled by Airflow and it stores its output in Delta Lake. Snowflake is where analysts and downstream reporting look at results, so the job's output has to be easy to load from there.

In scope:

- Reading the stockout probabilities produced by the prediction stage.
- Combining them with the inputs needed to size an order, such as current stock position, pack sizes, supplier constraints and lead times as the job sees them.
- Producing one proposed order line per store, item and supplier combination that needs one.
- Writing those lines to `replenish_orders`, partitioned by `order_date`.
- Making reruns safe, so that a retry or a manual rerun for a day does not double up lines.

Out of scope:

- Training or scoring the stockout model. The job treats probabilities as an input and does not second-guess them beyond sanity checks.
- Sending orders to suppliers or to a retailer's ordering system. Proposed orders are reviewed first. How analysts see and act on them is covered by the related note [[analyst-orders-serves-stores]].
- Long-term forecasting of demand for assortment planning. This job is about near-term replenishment.

The word proposed matters. Analysts are the users, and they are expected to override, trim or reject lines. The design keeps that in mind everywhere: the table holds proposals with enough explanation attached that an analyst can see why a line exists.

## Inputs

The main input is the set of stockout probabilities. Each row says, in effect, that for this store and this item the estimated chance of a stockout over the planning horizon is some value. The job does not recompute that value. It reads it as published by the upstream stage, and it uses the run date or the as-of date carried with the data to decide which probabilities are current.

The job also needs supporting inputs to turn a probability into a quantity:

- Inventory position per store and item, meaning on hand plus anything already on order or in transit. Without the in-transit part the job would reorder things that are already coming.
- Item and supplier reference data: pack or case size, minimum order quantities, and which supplier or distribution point serves a given store.
- Lead time information, so that an order proposed today is sized for the period it will actually cover rather than the period starting today.
- Any business rules the chain supplies, such as items that must not be auto-proposed or stores that are closed or in a freeze.

All of these are read as tables, mostly Delta. Some reference data originates in Snowflake and is landed into the lake by other pipelines. The job should not query Snowflake directly during its run. That is a deliberate choice: it keeps the job's runtime independent of warehouse load and makes reruns reproducible against the lake state.

### Input freshness

The most common source of bad proposals is stale input, not bad logic. If the probabilities are from an earlier day than the inventory snapshot, or the other way around, the job will happily produce lines that look plausible and are wrong. So the job checks that its inputs agree on the as-of date and fails the run when they do not, instead of picking whatever is latest from each. A loud failure is preferred to a quiet misorder. Airflow is responsible for ordering the upstream tasks so that the inputs are ready before this job starts, but the job verifies for itself rather than trusting the DAG.

## Order generation logic

The logic is deliberately simple and explainable, because analysts have to defend the numbers to category managers and store operations.

First, select candidates. A store and item pair becomes a candidate when its stockout probability crosses a threshold that is configurable and may differ by item class. Pairs below the threshold produce no line. This keeps the table small and keeps analysts from wading through noise.

Second, size the need. For a candidate, the job estimates the quantity needed to cover the horizon, given expected demand, the current inventory position and the lead time. The probability does not directly set the quantity. It sets whether to order and how much safety margin to carry. Higher risk gets a larger margin, within limits. The reasoning is that a probability is a statement about uncertainty, and the quantity should follow from demand and position, with the probability tuning the buffer.

Third, apply constraints. Raw quantities are rounded to pack sizes, raised to supplier minimums where required, and capped where a cap exists, for example shelf capacity or a maximum order size. When a constraint changes the raw need, the line records that it did, so the analyst can see the difference between what the model wanted and what is being proposed.

Fourth, attach explanation fields. Each line carries the stockout probability that triggered it, the inventory position used, and a short reason code for any adjustment. These are what make the proposal reviewable. Dropping them to save space was considered and rejected, because without them analysts cannot trust the table and will rebuild the logic in spreadsheets.

### Things the logic does not try to do

It does not optimize across stores for truck fill or delivery consolidation. It does not net demand between stores that share a distribution point beyond what the inputs already reflect. It does not learn from analyst overrides within the run. Those may be worth doing later, but each would make the output harder to explain, and the first priority has been an output analysts will use.

## Output table and partitioning

The output table is the Delta table `replenish_orders`. It is partitioned by `order_date`. This was chosen for these reasons:

- Analysts and downstream jobs nearly always ask about orders for a particular day, so filtering on `order_date` prunes to a small slice of data.
- Each run produces the orders for one date, so a run maps onto one partition. That makes overwrite of a single day simple and cheap.
- Retention and cleanup are easy to reason about by date.
- Loading into Snowflake, or exposing the table there, can proceed date by date, which fits how reporting is consumed.

The cost of this choice is that stores and items are not partition keys, so a query for one store across many days scans many partitions. For analyst use that has been acceptable. If it becomes a problem, prefer clustering or data layout optimization within the existing partitioning over changing the partition column. Changing the partition column of a live Delta table means rewriting it, and downstream readers have come to depend on `order_date` being the partition.

The row grain is one proposed order line. The table is append-and-replace by date in practice: a run for a given date replaces what that date held. Details of that are in the next section.

### Schema discipline

Delta enforces schema on write, and the job relies on it. New columns are added deliberately and not through automatic schema merging in production runs. The reasoning is that silent schema drift in a table that feeds analyst tools and warehouse loads causes confusing breakage far from the cause. When a column has to be added, change the job, add the column to the table in a controlled step, and tell the owners of the consumers, including whoever maintains the serving side described in [[analyst-orders-serves-stores]].

## Idempotency and reruns

Runs fail and are retried, and analysts sometimes ask for a rerun after a late upstream fix. So the write has to be safe to repeat. The design is a replace of the partition for the run's `order_date`, done as a single Delta transaction, rather than a plain append. With a plain append, a retry would duplicate every line for the day, and analysts would see double quantities.

Practical consequences:

- A rerun for a date fully replaces that date's proposals. Anything the rerun no longer proposes disappears, and anything new appears.
- Other partitions are not touched. A rerun for one date cannot damage another date.
- Because the replace is transactional, readers see either the old version of the date or the new one, never a half-written mix. Delta's snapshot isolation is what gives this, and it is the main reason Delta was chosen for the output.
- Time travel on the table gives a way back if a bad run replaced good proposals. Use it to inspect or restore, but remember that retention of old versions is finite.

One trap: if analysts have already started reviewing a date and a rerun replaces it, their view changes under them. If review state is stored elsewhere and keyed by line, a rerun can orphan it. The current approach assumes the job is run before analysts start their day, and reruns after that are a manual, communicated action. Do not schedule automatic reruns of a date once it may be under review.

## Scheduling and operations

Airflow runs the job once per planning day, after the upstream prediction stage has finished and its output has landed. The dependency is expressed in the DAG, and the job also checks as-of dates itself, as described under inputs. The job is a Spark application, so cluster sizing and shuffle behavior matter. The heavy steps are the joins between probabilities, inventory and reference data. Those are keyed by store and item, so skew is the thing to watch: a few very large stores or very popular items can concentrate work in a few tasks.

Operational habits that have helped:

- Look at row counts per run. A run that proposes far fewer or far more lines than usual is more often an input problem than a real change. The job logs counts at each stage so the point where the number jumps is easy to find.
- Keep the thresholds and constraint settings in configuration, not code, so that tuning does not need a release. Record what was in force for a given run, so a surprising order can be explained later.
- When the job fails on the as-of date check, do not override the check to get a run through. Fix the late or mismatched input and rerun.
- When a run is late, tell the analysts. They would rather know than find out from an empty table.

### Failure modes seen or expected

Empty or partial probability input leads to a short table that looks like a quiet day. The job should treat an input far smaller than normal as an error. Duplicate keys in reference data lead to doubled lines through a join, so reference tables are checked for uniqueness on their keys before joining. Items with missing pack data fall through constraint rounding and produce odd quantities, so they are flagged and excluded with a reason, not guessed. A change in the upstream probability scale, for example from a fraction to a percentage, would push everything over the threshold, and a sanity check on the range of the probabilities is the guard against that.

## Consumers and the Snowflake side

The table is read by two kinds of consumers. One is the serving path that shows proposals to analysts, described in the related note. The other is reporting in Snowflake, where `replenish_orders` is made available so that analysts and managers can slice proposals by store, supplier, category and date. The `order_date` partition is the natural handle for both.

Because there are consumers outside the job's own team, treat the table as a contract. Column names, types and meaning should not change casually. Where a change is needed, prefer adding to the table over altering what exists, and announce it. Deleting or renaming columns is the kind of change that breaks a Snowflake view or an analyst dashboard without warning.

## Decisions and open questions

Decisions taken so far:

- Output is a Delta table, `replenish_orders`, partitioned by `order_date`, with one run replacing one date.
- Proposals carry explanation fields so analysts can review them.
- The job does not query Snowflake at run time and works only from lake data.
- The job fails on mismatched input dates rather than choosing the latest of each.
- Schema changes are explicit, not automatic.

Open questions worth revisiting:

- Whether review state should be tied to proposal lines in a way that survives a rerun, so that late fixes need not disturb analysts' work.
- Whether store-level or supplier-level data layout within partitions would help the reporting queries that span many dates.
- Whether some cross-store logic, such as consolidation at a shared distribution point, is worth the loss of explainability.
- Whether threshold tuning should be per category by default instead of only where someone asked for it.

If you change any of the decisions above, update this note in place and say why, so the next person does not have to reconstruct the reasoning.
