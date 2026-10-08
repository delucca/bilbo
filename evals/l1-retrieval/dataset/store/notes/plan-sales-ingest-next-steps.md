---
id: 01M27D9X9H16BHKS3ARD3AKYEX
created: 2026-09-11T01:58-03:00
---

# sales-ingest-pipeline: next steps

Working plan for sales-ingest-pipeline, written quickly so the next session does not have to rebuild the picture. It covers what to look at first, what to tighten, and what to leave alone. Nothing here is a settled decision; each item needs a check against the actual code and the actual data before it turns into work.

The pipeline takes point-of-sale data from the grocery chains, lands it in Delta Lake tables through Spark jobs written in Scala, is scheduled by Airflow, and feeds Snowflake for the analysts. The stockout model downstream depends on it being complete and on time. The model side is in [[stockout-model-spark-trains]], so keep model concerns there and only note the contract between the two here.

## Where things stand

The pipeline works well enough that analysts use the output daily. The weak spots are the ones you would expect: late files from a few chains, uneven handling of corrections and returns, and not much visibility when a run is partly wrong but still green. The first job is to confirm that picture against the DAGs, the job logs and a few recent runs, not to trust this paragraph.

Before changing anything, read through the ingest jobs end to end once. Write down which steps are idempotent and which are not. That list drives most of the ordering below.

## Reliability of the landing step

Raw sales files arrive in different shapes and at different times depending on the chain. The landing step should be safe to rerun for any partition without creating duplicates. Check how it handles a file that shows up twice, a file that shows up late after downstream tables were already built, and a file that is truncated.

Things to do here:

- Confirm that writes into Delta use a merge or overwrite by partition rather than blind appends.
- Look at how the job decides a file is complete before reading it.
- Make sure a failed run leaves the table in the previous good state, which Delta should give us if the commit is atomic.
- Keep a quarantine location for files that fail parsing, so one bad file does not block a whole chain.

Do not redesign the file layout yet. Fix the rerun safety first and see what that shows.

## Schema and data quality checks

Schemas drift. A chain adds a column, changes a type, or sends a code we have not seen. Right now it is unclear how much of that is caught at read time and how much leaks into the tables.

Plan:

- Decide where schema enforcement lives (read, write, or both) and write it down in this note once chosen.
- Add checks for the basics: missing store identifiers, missing product identifiers, negative or absurd quantities, timestamps in the wrong range, and unexpected gaps in a store's daily sales.
- Separate checks that should fail the run from checks that should only warn. Failing too eagerly will make analysts lose a day of data over a cosmetic problem.
- Store check results in a table so trends are visible, not only the latest run.

Thresholds are for the team and the analysts to set. Do not guess them in code and leave them there.

## Corrections, returns and late data

Sales corrections and returns affect the stockout signal, since a return can look like negative demand. Find out how the current jobs treat them, and whether that treatment is the same for every chain. If it differs, list the differences before trying to unify them.

Late-arriving data needs a clear rule: how far back the pipeline reprocesses, and what happens to downstream tables that already consumed the older version. Check whether Delta time travel or change feeds can help downstream jobs pick up only what changed, instead of rebuilding everything.

Talk to a merchandising analyst about which corrections matter to them before encoding anything. The pipeline should preserve the original records so the treatment can change later without a re-pull from the chains.

## Orchestration in Airflow

The DAGs should say clearly what depends on what. Look for places where tasks are tied together by timing or by sleeping instead of by a real dependency such as a table or partition being ready.

Items to review:

- Retries and backoff on each task, and whether a retry can double-write.
- Alerting: who is told, through what channel, and whether the message says which chain and which partition failed.
- Backfill behavior. A backfill should be a normal, documented operation and not a hand-edited run.
- Task granularity. One giant task is hard to rerun; hundreds of tiny ones make the scheduler slow. Aim for a middle ground per chain or per group of chains.

If a DAG change would alter run times for downstream consumers, tell the model side before shipping it.

## Delta to Snowflake handoff

The analysts see Snowflake, so that is where problems become visible to users. Check how data moves across: full reload, incremental by partition, or something else. Make sure a rerun upstream produces a correct refresh downstream and does not leave stale rows.

Also compare a sample of rows and totals on both sides on a regular basis. A simple reconciliation job that reports differences is worth more than a clever load. Keep it read-only at first.

Look at cost and runtime too, but only after correctness. Partitioning and clustering choices on the Delta side and the Snowflake side may not match what the queries actually do, so look at real query patterns before changing either.

## Observability and housekeeping

Run-level metrics are needed: rows read, rows written, rows rejected, lateness per chain, and time per stage. Put them somewhere queryable and add a small dashboard or a saved query that the on-call person can open.

Housekeeping on Delta tables also needs an owner: file compaction, cleanup of old files, and retention for raw data. Check what is scheduled now and what is done by hand. Any retention change should be agreed with whoever owns the data agreements with the chains, not decided inside this component.

Last, update the runbook. A short page covering how to rerun one chain, how to backfill, and where to look first when numbers look wrong would save more time than most code changes on this list.

## Order of work and open questions

Suggested order: read-through and idempotency list first, then landing safety, then quality checks, then late data and corrections, then the Airflow cleanup, then the reconciliation with Snowflake, then metrics and housekeeping. The order can change if the read-through turns up something urgent.

Open questions to resolve with people, not with code:

- Which chains cause the most trouble, and is there a pattern in how they send data?
- What do analysts need when data is late: a partial view now, or a complete view later?
- Who owns retention and compaction settings?
- How much change can the downstream model absorb without retraining or re-checking?

Update this note as answers come in, and move anything that becomes a firm decision into its own note instead of leaving it buried here.
