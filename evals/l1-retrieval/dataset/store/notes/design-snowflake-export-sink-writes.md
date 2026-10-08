---
id: 01JT1ADC601TG3EXHX8NEATMJ7
created: 2025-04-29T14:44-03:00
sources:
  - "code: src/main/scala/shelfsense/export/SnowflakeSink.scala"
---

# snowflake-export-sink design

snowflake-export-sink is the last step of ShelfSense that touches order data. It takes the replenishment order proposals the Spark jobs produce and puts them where merchandising analysts can read them, in Snowflake. Internally the team calls it `coldbridge`, so you will see that name in chat, old tickets and some dashboards. It is the same component as `snowflake-export-sink`. This note uses the long name.

## Purpose

Analysts do not read Delta Lake tables directly. They work in Snowflake, so every proposal the pipeline generates has to show up there, once, in its latest form. snowflake-export-sink is the only writer for that data. If an analyst sees a wrong or missing proposal, this is the first place to look after the proposal generation itself.

## Target table

The sink writes proposals to `SHELFSENSE.PUBLIC.ORDER_PROPOSALS`. Nothing else in ShelfSense should write to that table. Treat it as owned by the sink: schema changes go through the sink code, not through ad hoc DDL.

## Write mechanism

The sink uses the Spark Snowflake connector from the Scala Spark job. It does not append rows. It runs a `MERGE` keyed on `order_id`. A proposal that already exists is updated in place, and a proposal that does not exist is inserted. The connector first lands the DataFrame in a temporary staging table, then the `MERGE` runs against the target.

```sql
MERGE INTO SHELFSENSE.PUBLIC.ORDER_PROPOSALS
  USING <staging table>
  ON target.order_id = source.order_id
  WHEN MATCHED THEN UPDATE ...
  WHEN NOT MATCHED THEN INSERT ...
```

The sketch is shortened on purpose. The column lists in the real statement come from the proposal schema in the job.

## Why MERGE on order_id

Airflow retries tasks, and the proposal job can be rerun for the same planning cycle. A plain append would then create duplicate proposals, and an analyst could approve the same order twice. Keying on `order_id` makes the write idempotent: running the sink again with the same input leaves the table the same. It also lets a regenerated proposal replace an older version of itself instead of sitting beside it.

This only works if `order_id` is stable. The id must be derived from the same inputs on every run, and not from a timestamp or a random value. If the id generation changes, reruns will stop matching and duplicates will return.

## Upstream and scheduling

The input is the proposals output written as Delta Lake tables by the earlier stages. An Airflow task runs the sink after proposal generation has finished and its checks have passed. The sink should never run on partial proposal output, so the dependency on the generation task is strict, not best effort.

## Failure and retry behavior

The `MERGE` runs as a single statement, so a failure leaves the target either fully updated or untouched for that run. Because of the key, the normal response to a failed run is to retry the Airflow task. No manual cleanup of the target is needed in that case. If a retry keeps failing, check the Snowflake side first (credentials, warehouse availability, permissions on the target) before suspecting the data.

## Duplicate keys in the source

A `MERGE` in Snowflake can behave badly when several source rows match the same target row. The sink assumes one row per `order_id` in its input. If proposal generation ever emits two rows with the same `order_id`, the result may be nondeterministic or the statement may fail, depending on the session settings. Deduplicate upstream and fix the cause there. Do not hide it inside the sink.

## Things that are easy to get wrong

- Do not point the sink at a different table by editing a hard-coded name in one place. Check how the target is configured in the job before changing it.
- Do not switch the write mode to append to "speed things up". It breaks the idempotence described above.
- Do not call the component by the codename in code or config names. The codename is only a nickname.
- Do not let other jobs write into the same table. A second writer defeats the single-owner assumption.

## Open questions

- Whether rows for proposals that were withdrawn upstream should be deleted from the target or marked. At the moment the `MERGE` has no delete branch, so withdrawn proposals stay until something else cleans them up.
- Whether the sink should report row counts for inserted and updated rows to Airflow so analysts can see how much changed per run.

## Where to look

Start with the sink's Scala code in the Spark project and the Airflow task that calls it. For data questions, query `SHELFSENSE.PUBLIC.ORDER_PROPOSALS` in Snowflake and compare by `order_id` against the Delta Lake proposals for the same cycle. If the two disagree, the problem is either in the sink run or upstream of it, and comparing the key sets usually shows which.
