---
id: 01K6AXBH1TM888458Z54H3Y51S
created: 2025-09-29T11:17-03:00
sources:
  - "doc: rollup benchmark log"
---

# rollup-worker concurrency benchmark: database is the ceiling

Benchmarking found that rollup-worker throughput stopped improving above `--concurrency 4` because the database became the bottleneck. Past that setting, extra worker processes did not finish rollups any faster. They mostly waited on the database. This note records what that means for ClassroomCompass, how to read the result, and what to do before anyone raises the setting again.

```
rollup-worker --concurrency 4
```

That is the setting as it is referred to in this note, not a full launch command. Check the actual deployment config for the real invocation.

## The finding in plain terms

rollup-worker is the Celery worker in ClassroomCompass that turns raw student activity into progress summaries against curriculum standards. Those summaries feed the teacher dashboards and the exercise suggestions. During benchmarking we raised the worker's concurrency step by step and measured how much rollup work got done per unit of time.

Throughput rose as concurrency went up, until `--concurrency 4`. Above that value the curve went flat. More processes did not give more completed rollups. The reason was the database: once the worker processes together issued enough queries and writes, the database could not serve them any faster, and the extra processes only queued behind each other.

If you remember one thing: `--concurrency 4` is where the gain stops for rollup-worker, and the limit is the database, not the CPU of the worker host and not Celery itself. Raising the value further costs memory and database connections and returns nothing in throughput.

## What was measured and how to read it

The benchmark ran rollups over a realistic body of student progress data and varied only the worker concurrency. The measure that mattered was how many rollup tasks completed over a fixed period. We also watched where the time went inside each task.

A few points about reading the result:

- The flat part of the curve is the important part. Below `--concurrency 4` the worker was the limit, and adding processes helped. Above it the database was the limit.
- The result says nothing bad about the rollup code itself. The tasks were not slow because of the Python. They were slow because they waited on database round trips and on contention between concurrent writers.
- The benchmark was about throughput, meaning total work finished. It was not mainly about the latency of a single task. A single task did not get faster or slower in a way that changes the conclusion.
- The result is tied to the database as it was configured during the benchmark. A larger or differently tuned database could move the ceiling. The number is a property of this combination of worker, queries and database, not a law.

When the setup changes, the number can change. Treat `--concurrency 4` as the measured answer for now, and re-measure if the database, the schema, or the rollup queries change in a meaningful way.

## Why the database is the limit

Every rollup reads a student's recorded activity, works out progress per curriculum standard, and writes the summary back. Many tasks running at once means many concurrent readers and writers on the same tables. Several things can then happen, and they usually stack:

- Connections are finite. Each worker process holds its own connection, and the database has to schedule work from all of them.
- Writers touching the same or neighbouring rows contend for locks, so extra writers spend time waiting rather than working.
- Read-heavy aggregation competes with the writes for the same I/O and cache.
- Past a point, the database spends effort juggling concurrent sessions instead of finishing queries, so adding sessions can make each one slower.

We did not isolate which of these dominated, and this note does not claim to. The benchmark showed that the database was the limiting resource at the point where the curve flattened. It did not break that down into lock waits versus I/O versus connection handling. That is a follow-up if someone wants to push the ceiling higher.

## What this means for configuration

Keep rollup-worker at `--concurrency 4` unless there is new measurement saying otherwise. Some practical consequences:

- Do not raise concurrency to clear a backlog of rollup tasks. It will not clear faster, and it adds load on the database that other parts of ClassroomCompass share. Teachers loading dashboards use the same database, so a rollup surge can make the interactive pages slower.
- If rollups feel too slow, the fix is on the database side or in the queries and batching, not in more worker processes.
- Running more rollup-worker instances on more hosts does not get around this. The limit is shared, so many hosts at modest concurrency hit the same wall as one host at high concurrency. Think of the total number of concurrent rollup sessions against the database, not the value on each host.
- Lowering concurrency below `--concurrency 4` is a reasonable choice if the database needs headroom, for example during a heavy period for the interactive site. The cost is some throughput, since below the knee the worker is the limit.

If the deployment tooling sets concurrency in more than one place, such as a process manager file, a container spec and an environment default, make sure they agree. A mismatch is an easy way to end up running more than intended without noticing.

## Things worth trying before raising the limit

These are ideas, not tested results. None of them were measured in this benchmark.

- Look at the rollup queries and the indexes behind them. If tasks do a lot of repeated reads, caching or reading in bulk may remove a large share of the database load.
- Batch writes. Fewer, larger writes usually cost the database less than many small ones, and they reduce lock churn.
- Check whether tasks that touch the same student or the same class are being run at the same time. Routing related work so that it does not overlap can cut contention.
- Consider whether some of the aggregation could be done elsewhere. Elasticsearch is already part of the stack for search, and it may suit some read-heavy summaries better than the relational database. That is a design question and needs its own evaluation, because it adds a second store to keep consistent.
- Look at connection handling, such as pooling and how long tasks hold connections. A task that holds a connection while doing non-database work wastes a scarce resource.
- Measure the database side directly while the benchmark runs: lock waits, slow queries, I/O. That would turn the general statement here into a specific cause.

If one of these is tried, rerun the same concurrency sweep afterwards. The point of the sweep is to find the new knee. If the knee moves above `--concurrency 4`, update this note with the new value and the change that caused it.

## How to repeat the benchmark

The approach is simple, and the details matter less than keeping it consistent between runs:

- Use the same body of input data each time, so runs are comparable.
- Change only the concurrency of rollup-worker between runs. Leave the database settings, the task code and the host alone.
- Run long enough that start-up effects and warm caches do not dominate the numbers.
- Make sure nothing else heavy is using the database during the run. Shared load will blur the knee.
- Record completed tasks per period for each concurrency value, and look for where the increase stops.
- Note the database configuration and the data volume with each result, since both affect where the ceiling sits.

A result that shows throughput still rising well above `--concurrency 4` would mean something changed, most likely on the database side, and deserves a closer look rather than a quick config change.

## Caveats and open questions

- The ceiling was found under benchmark conditions. Production has other traffic, different data sizes and different timing, so the real knee could sit somewhat differently. `--concurrency 4` is the measured, defensible default, not a guarantee for every school's data volume.
- Data volume grows over a school year as more student activity is recorded. Queries that were cheap early on may get heavier. Re-check the number at the busiest time of the year, not only at the start.
- We have not separated the cost of the aggregation reads from the cost of the summary writes. Knowing which dominates would tell us whether to work on queries or on batching first.
- We have not tested how rollup-worker concurrency interacts with other Celery workers in the project that also use the database. If several worker types run together, their combined load is what the database sees, and the right value for rollup-worker could be lower when those are busy.
- scikit-learn based exercise suggestion reads the output of the rollups. A slow or lagging rollup means suggestions can be based on stale progress. That is a reason to care about rollup throughput, and also a reason not to solve it by overloading the database, since the suggestion step depends on that database too.

## Short version for a later session

rollup-worker stops getting faster above `--concurrency 4`, and the database is why. Do not raise the value to fix slowness. Look at the queries, batching, contention and database capacity first, then re-run the concurrency sweep to find the new knee. Update this note if the number changes.
