---
id: 01K81RG8QMXQMBVGTREQDG6ZB0
created: 2025-10-20T18:30-03:00
---

# snowflake-export-sink: October load test result

The snowflake-export-sink merged 95,000 rows in 3.5 minutes during the October load test. That is the number to compare against next time. It was one run, so treat it as a rough baseline and not a guarantee. This note records what the run showed, what it did not show, and what to check before trusting it for real replenishment loads.

## What the test measured

The test pushed a batch of prediction and replenishment rows from the Delta Lake side through snowflake-export-sink and into Snowflake, using a merge so that reruns update existing rows instead of duplicating them. The headline figure is the full wall time for the merge step: 95,000 rows in 3.5 minutes. The clock covered staging the data and running the merge. It did not cover the upstream Spark job that builds the rows, and it did not cover the Airflow scheduling delay before the task started.

## Why the number matters

Merchandising analysts want replenishment orders early in the morning. The export is one of the last steps in the Airflow DAG, so any slowness there delays the orders directly. A merge of 95,000 rows in 3.5 minutes is comfortably inside the window we have for a single chain. The open question is how it behaves when several chains export at the same time, which this test did not try.

## Conditions of the run

The run used a test dataset shaped like production output for one mid-sized grocery chain: store-level rows with a product key, a store key, a predicted stockout risk, and a suggested order quantity. The Snowflake warehouse was a normal-sized one and nothing else heavy was running on it. The Spark side was Scala code on the usual cluster settings. We did not tune anything specially for the test, so the figure reflects defaults plus the current sink code.

## Gaps and caveats

- Only one run was recorded, so there is no spread or variance to quote.
- The test did not overlap with other warehouse load, which is the normal situation in the morning.
- Row width was typical, not worst case. Wider rows or many more columns could change the time.
- The test did not check what happens when most rows already exist versus when most are new. Merge cost can differ between those cases.
- Failure and retry behavior was not part of this test.

## What to do next

Repeat the run at least a few times to get a range, and write the range here next to the first result. Then run it with two or three chains exporting at once, and with a warehouse that is busy with other queries. If the time grows a lot, look at how the sink stages data and how many merge statements it issues before touching the warehouse size. Also add the merge duration as a metric in the Airflow task so we stop depending on one-off tests.

## Pointers

The sink code lives in the export module of the ShelfSense Scala project. The Airflow task that calls snowflake-export-sink is the final export step of the replenishment DAG. When updating this note, keep the original figure of 95,000 rows in 3.5 minutes and add new runs beside it instead of replacing it, so the trend stays visible.
