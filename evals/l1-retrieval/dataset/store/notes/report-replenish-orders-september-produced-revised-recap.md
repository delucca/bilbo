---
id: 01KW2W28VP3MBST4HW3MJ0FBXV
created: 2026-06-26T18:06-03:00
---

# replenish-orders-job: September orders produced and revised

Quick note on how the September replenishment run went for replenish-orders-job. The first batch of orders came out, analysts looked at it, and a revised batch was produced afterwards. Writing this down so nobody has to reconstruct it from Airflow history.

## What the job does

replenish-orders-job is the Scala Spark job that reads the stockout predictions from Delta Lake and turns them into replenishment orders per store. The output goes to Snowflake, where the merchandising analysts pick it up. Airflow schedules it after the prediction tables are refreshed.

## First September run

The first run finished normally and produced orders for the usual set of stores. Nothing failed in Airflow. The problem showed up later, when analysts compared order quantities against what they expected for a few categories. Some quantities looked too high for stores with low shelf capacity.

## Why it was revised

The cause, as far as I could tell, was the order sizing step using inputs that were not as fresh as intended. The job picked up an older snapshot of one of the Delta tables, so the predictions did not match the latest stock levels. I did not confirm every detail of this. Worth checking the table version the job read against the one it should have read.

## Revised run

After the input was fixed, the job was rerun and the orders were regenerated. The revised output replaced the first batch in Snowflake. Analysts were told which batch to use. The volumes in the revised batch were closer to what they expected, and the caps per store were respected this time.

## Things to watch

- Check which Delta snapshot the job reads before trusting a run.
- Reruns overwrite the earlier batch, so keep a copy if someone needs to compare the two.
- The configured order limits per store should be checked after any rerun.
- Airflow showing green does not mean the quantities are right.

## Handy check

Something like this to compare the versions of the Delta table (table name is a placeholder, adjust it):

```sql
DESCRIBE HISTORY predictions
```

## Open questions

- Should the job fail loudly when the input snapshot is older than the usual freshness window?
- Is there a better way to mark which batch is current in Snowflake, so analysts do not guess?
- The sizing step may need a sanity check against shelf capacity before writing output.
