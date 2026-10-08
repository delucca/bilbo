---
id: 01KER7A1PC24R0BVVX0YYH71RG
created: 2026-01-12T01:26-03:00
---

# sales-ingest-pipeline: general direction

We settled on a general direction for sales-ingest-pipeline and this note keeps the reasoning so nobody has to redo the argument. It is deliberately loose. It says which way we go and why. It does not fix thresholds, schedules or sizes, because those change with the chains we onboard and belong in config, not in a decision note.

The short version: sales-ingest-pipeline lands raw point-of-sale data first, untouched, then builds cleaned and conformed tables from it in separate steps. Spark does the heavy work, Delta Lake holds every stage, Airflow orders the steps, and Snowflake only receives the final curated tables that analysts and the replenishment logic read. Each of those roles stays narrow.

## Why this shape

The stockout model is only as good as the sales history it sees. Grocery chains send data in uneven ways: late files, repeated files, corrected transactions, stores that go quiet for a while and then send a burst. If we clean on the way in, we lose the ability to explain why a number changed. If we keep the raw layer as it arrived, we can always rebuild downstream tables after a bug fix or a rule change, and we can answer an analyst who asks why a store looked empty on a given day.

So the raw layer is append-only in spirit. We do not fix records in place there. Fixes happen in later layers, and those layers can be recomputed from the raw one. This was the main thing we agreed on, and the rest follows from it.

Reprocessing must be safe. Running a step twice for the same input should give the same output. We prefer designs where a rerun replaces or merges a clearly bounded slice of data instead of adding to it. When a design makes reruns scary, we treat that as a defect in the design, not as something operators should just be careful about.

## Layers and who owns what

Three broad layers, no more unless a real need shows up.

- Raw: the data as delivered, plus minimal bookkeeping about when and from where it came. Nothing is dropped here, including rows we suspect are bad.
- Conformed: types fixed, duplicates resolved, store and product identifiers mapped to our own keys, returns and voids handled consistently. This is where the business rules live and where most review effort goes.
- Curated: aggregates shaped for the stockout features and for analyst reporting. This is the only layer that is published to Snowflake.

Spark jobs own the move from one layer to the next. Airflow owns ordering, retries and alerting, and holds no business logic. If a DAG file starts to contain data rules, that logic gets moved into the Spark code. Scala is the language for the jobs; we are not adding a second language for transformations.

Delta Lake gives us table history and merge semantics, and we lean on both. We use table history to investigate odd data and to roll back a bad run when needed. We do not treat history as a long-term archive, because retention is limited and the raw layer is the real source of truth.

## Handling bad and late data

The direction is to keep going and make problems visible, not to halt on the first oddity. A single chain sending a malformed file should not block everyone else. Bad records go to a quarantine area with enough context to diagnose them, and the run reports how much was quarantined. A large quarantine share for one source is an alert for a human, not something we hide.

Late and corrected data is normal. The conformed and curated layers are expected to absorb it by recomputing the affected slices, within a window that we choose per source and keep in configuration. Beyond that window, changes are handled by an explicit backfill that someone starts on purpose, so that quiet history rewrites do not surprise the model team.

We want freshness and correctness to be tradeoffs we can see. When analysts need data early in the day, we accept that early numbers may later be revised, and we label the curated outputs so a reader can tell provisional from settled. We prefer that to delaying everything until it is perfect.

## What we chose not to do

- No streaming rewrite for now. Batch with frequent runs meets the need, and the team can reason about it. We will revisit only if a customer need clearly cannot be met in batch.
- No business rules in Airflow, and no direct writes to Snowflake from the early layers.
- No per-chain forks of the pipeline. Differences between chains are handled with source-specific adapters at the edge of the raw to conformed step, so the core stays shared.
- No manual edits to data in any layer. If something is wrong, we fix the rule and recompute.

## Open points

These are not decided and should not be read as decisions.

- How long to keep raw data for chains that stop sending, and who signs off on deleting it.
- Whether the quarantine area deserves a small review tool, or whether queries are enough.
- How to expose the provisional versus settled label to analysts so it is hard to miss.
- Whether the curated layer should be split by use, one for modeling and one for reporting, if their needs drift apart.

## How to use this note

When changing sales-ingest-pipeline, check the change against the direction above: raw stays untouched, rules live in Spark, Airflow only orders and retries, reruns are safe, bad data is surfaced and not hidden. If a change breaks one of those, either rethink it or update this note with the reason. Concrete settings such as windows, schedules and limits live in configuration and the job code, and this note should not be edited to chase them.
