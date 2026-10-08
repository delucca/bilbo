---
id: 01KMZCATMHYZG031V9DFTJHJ0Z
created: 2026-03-30T09:43-03:00
---

# replenish-orders-job: things to watch when changing it

Notes for anyone touching replenish-orders-job. It turns stockout predictions into replenishment orders, and the people reading the output are merchandising analysts who will act on it. A quiet mistake here becomes a wrong truck order at a store, so most of the traps below are about silent wrongness, not crashes.

## Inputs and upstream timing

replenish-orders-job reads predictions and inventory state that other jobs produce. The Airflow schedule decides when it runs, but it does not guarantee that the upstream tables are finished. If you change the schedule or add a dependency, check that the job cannot start against yesterday's predictions or a half-written inventory snapshot. A late upstream usually does not fail the run. It just produces orders built on stale data.

Be careful when changing how the job picks "the latest" partition or version of an input. Reading a Delta table at the wrong version is easy to do and hard to notice. Look at what the job actually read before trusting a green run.

Also watch for schema drift. If the prediction output gains, renames or reorders a column, Spark may still run and fill nulls. Check null handling for every column you depend on.

## Order logic

Rounding is the usual source of bugs. Case packs, minimum order quantities, and store or supplier constraints all interact. Changing the order of these steps changes results. Rounding up before applying a cap gives a different answer than capping first. Test with a few stores that sit near the edges: very low demand, items with no recent sales, items with a pack size larger than the predicted need.

Do not assume predicted demand is non-negative or non-null. Decide explicitly what happens with a missing prediction. Skipping the item and ordering zero are different behaviors, and analysts will read them differently.

Be careful with Scala numeric types. Mixing integer and decimal arithmetic inside Spark expressions can truncate without warning. Keep quantity math in one consistent type and cast at the edges.

## Writes, reruns and idempotency

The job must be safe to rerun. Airflow retries and manual reruns happen, and a rerun should replace the earlier output for the same run, not append to it. If you change how output is written to Delta Lake, check what a second run does. Duplicate orders for the same store and item are the worst outcome.

If the merge or overwrite keys change, think about rows that existed before the change. Old rows may no longer match the new key and will sit next to the new ones. Partition overwrite settings also matter: a dynamic versus static setting can wipe more than you meant to.

Snowflake is downstream. Whatever is loaded there may be consumed by reports or by other systems. Changes to column names, types or grain need to be coordinated with whoever reads that data. Do not treat the Delta output as private.

## Performance and Spark behavior

Store-by-item data skews. A few large stores or fast-moving items can dominate a partition. A join or aggregation that looks fine on a small sample can stall on production data. Be wary of adding wide joins, collecting to the driver, or UDFs that block optimizer pushdown.

Changing shuffle partitioning, caching or broadcast hints can change both runtime and, occasionally, which rows survive a deduplication step that depends on ordering. If dedup relies on "first" or "last", make the ordering explicit.

## Testing and rollout

Run a change against a recent real slice and compare the orders to the previous output, store by store. Look at totals, but also look at which items appeared or disappeared. Small logic changes often move many orders by a little, and that is hard to see in totals alone.

Prefer shipping risky changes behind a switch, or running the new logic alongside the old and diffing, before analysts see it. Tell the analysts when order behavior changes on purpose, since they will otherwise report it as a bug.

Last, check the job's config and the Airflow definition together. Parameters are sometimes set in both places, and changing only one leaves the job running with the old value.
