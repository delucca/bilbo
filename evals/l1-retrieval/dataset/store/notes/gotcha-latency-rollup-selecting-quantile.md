---
id: 01JVJRECT48AH9HEGRQBW6GTJD
created: 2025-05-18T19:32-03:00
---

# latency_rollup_mv: quantile states need the Merge combinator

If you select quantile columns straight out of latency_rollup_mv and get `Code: 43. DB::Exception: Illegal type AggregateFunction`, nothing is broken. The columns hold partial aggregate states, not numbers, and you have to read them with the Merge combinator. This note is the short version of why, how to spot it, and what to do. It also records the old name, because people keep searching for it.

Naming, stated plainly: rollup1m was the previous name of latency_rollup_mv; the component is called latency_rollup_mv now. If you find rollup1m in an old dashboard, a runbook, a saved query, a Slack thread or a Helm values file, it means latency_rollup_mv. Use latency_rollup_mv in anything new, and fix the old name when you touch the file.

## The symptom

The error text is `Code: 43. DB::Exception: Illegal type AggregateFunction`. It shows up when a query reads the quantile columns of latency_rollup_mv without the Merge combinator. Typical places it appears: a hand-written query in the ClickHouse client, a new Grafana panel built in a hurry, a Java service that issues a plain select against the rollup, or a notebook someone uses to compare latency before and after a deploy.

The message is confusing because it reads like a schema problem or a driver problem. People look at column types, try casting, try changing the client version, and lose an hour. It is none of those. The server is telling you that the column type is an aggregate function state, and that the operation you asked for does not accept that type.

Sometimes the error is not raised at all and you get something worse: the client returns opaque binary-looking values for the quantile columns, or a panel shows garbage or nothing. That depends on the client and on the output format. If a latency number looks like noise, check whether you are reading a state.

## Why it happens

latency_rollup_mv is a materialized view that rolls span durations up into small time buckets. Inserts into the raw trace table fire the view, and each insert produces a partial aggregate for its bucket. Quantiles cannot be added or averaged like counts, so the view stores the intermediate state of the quantile aggregate instead of a final value. Later, a query has to combine the states from many parts and many inserts, and only then turn the combined state into a number.

The state-producing side uses the State combinator when the view is defined. The reading side must use the matching Merge combinator. If you skip it, you are asking the server to treat a state column as an ordinary value, and it refuses with the error above.

This is standard ClickHouse behavior for the aggregating storage engines and not a bug in TraceQuill. The design is deliberate: it lets us roll up cheaply at insert time and answer latency questions over any window later, without keeping every span duration around for the query.

## What to do instead

Read the quantile columns with the Merge combinator, and group by the dimensions you care about. The Merge function takes the state column and returns the final quantile value for the group. Make sure the quantile level you ask for matches the level the view was defined with; the Merge function is parameterized the same way as the state function, and a mismatch gives wrong or failing results. Look at the view definition to see which levels it stores before writing a query.

If you want to sum counts or other plain columns in the same query, that works as usual. Only the state columns need the combinator. Mixing the two in one select is fine as long as the grouping is consistent.

When you want a longer window than the stored bucket width, do not try to average the per-bucket quantiles. Merge the states across the window and read once. Averaging per-bucket quantiles gives a number that looks plausible and is wrong, and it is the most common way regression summaries drift from reality.

If you need a final value many times, for example in a dashboard that refreshes often, consider a thin query wrapper that already applies the Merge combinator, so panel authors never touch the raw state columns. Do not add a second materialized layer without talking to the team; it multiplies storage and complicates backfills.

## How to recognize it fast

Check these in order when someone reports the error or odd latency numbers.

First, look at the exact message. If it is `Code: 43. DB::Exception: Illegal type AggregateFunction`, the cause is almost certainly a missing Merge combinator on a quantile column of latency_rollup_mv.

Second, look at the query text. Is the quantile column selected by name with no combinator around it? Is it passed to an arithmetic expression, a comparison, an order by, or a plain function? Any of those will trigger the error or a close relative of it.

Third, look at where the query came from. Panels copied from an older dashboard sometimes still point at rollup1m. Those either fail because the old name no longer exists or, if an alias lingers somewhere, hit the same state columns and fail in the same way. Fix the name and the combinator together.

Fourth, if the error appears only after a change, check whether someone replaced a final-valued table in a query with latency_rollup_mv. That swap is a common trigger, because the column names look similar but the types are not.

## The rename

The previous name was rollup1m. The current name is latency_rollup_mv. The rename was made so that the component says what it holds: latency, rolled up, and that it is a materialized view. The old name described only the bucket width, which says nothing about contents and is wrong the moment the bucket width changes.

What this means in practice:

Search for both names when you look for usages. A grep for the new name alone will miss stale references. Check repository code, dashboards exported from Grafana, alert rules, Kubernetes manifests and config maps, runbooks, and any scheduled jobs that issue queries.

Do not assume that a reference to rollup1m is dead. Some of them are still live in places that were not updated, and they may be failing quietly. Treat each one as a bug to fix, and point it at latency_rollup_mv with the Merge combinator in place.

When writing documentation or commit messages, use latency_rollup_mv. If you need to mention the old name so people can find the history, write that rollup1m was the previous name of latency_rollup_mv, and move on.

Do not reintroduce the old name as an alias without a reason. An alias would let stale queries keep running, and they would keep hitting the state-column error or, worse, produce numbers from a misused column.

## Where this bites in TraceQuill

The latency regression summary that runs after each deploy reads from latency_rollup_mv. It compares a window before the deploy to a window after it, per service and per operation, and flags the ones whose tail latency moved. That job must merge states across each window before comparing. If someone edits the job and drops the combinator, it fails loudly with the error above, which is the good outcome. The bad outcome is a change that keeps running but averages per-bucket values, which hides regressions.

Grafana panels used by site reliability engineers during an incident also read from latency_rollup_mv. Panel authors are often under pressure and copy a query from a neighbor. If the neighbor used the combinator, fine. If the neighbor used a raw table, the copy breaks. Keep a known-good panel query around and copy from it.

The Java services that expose latency endpoints to other tools run the same kind of query through the ClickHouse client. Those queries are in code and should be covered by a test that runs against a real ClickHouse instance, not a mock. A mock will never produce the type error, so it will never catch a missing combinator.

The OpenTelemetry collector side does not touch latency_rollup_mv directly. It writes raw spans, and the view fills itself. So collector changes do not cause this error, but changes to the raw table schema can change what the view produces, so review them together.

## Gotchas around the same area

Backfills. Because the view fires on insert, data that existed before the view was created is not in it. Backfilling means inserting state values for old data in the same state format, not final numbers. Inserting finals into a state column fails with a type error of its own. Do the backfill with the State combinator on the select side of the insert.

Part merges. States are combined in the background as parts merge, and that is invisible to queries as long as you use the Merge combinator. Do not rely on the number of stored rows per bucket; it changes over time and between replicas.

Replicas and distributed tables. If reads go through a distributed layer, the Merge combinator still has to be applied; the layer only gathers partial results from shards. Check that the layer does not apply a final step for you, since it will not.

Time zones and bucket edges. Buckets are aligned to the stored width in the server time zone. A window that does not line up with bucket edges includes whole buckets at the edges, so a window comparison can be off by a partial bucket. Keep before and after windows aligned to the same edges so the comparison is fair.

Retention. The view has its own retention, separate from the raw spans. A regression summary that looks far back may find nothing in latency_rollup_mv even when raw spans still exist, or the reverse. Check retention before blaming the query.

Schema changes. Changing the quantile levels or the dimensions in the view means creating a new view and migrating, because states with different parameters cannot be merged together. Do not alter in place and hope. Plan the migration, run both for a while, compare, then switch readers.

## Checklist before you ship a query

Does every quantile column from latency_rollup_mv go through the Merge combinator? Does the quantile level match the stored level? Are you merging across buckets instead of averaging per-bucket results? Are the windows aligned to bucket edges? Is the name latency_rollup_mv and not rollup1m anywhere in the query, the panel, or the config that holds it? Is there a test against a real ClickHouse that would fail if the combinator went missing?

If all of that is yes, the query is fine. If you still see `Code: 43. DB::Exception: Illegal type AggregateFunction`, read the full query again, including subqueries and views built on top of latency_rollup_mv. A subquery that selects the raw state column and a parent query that applies a plain function to it will fail the same way, and the error does not always point at the right line.

## Quick answers

What does the error mean? A state column was used where a final value is needed. Add the Merge combinator.

What is the exact error? `Code: 43. DB::Exception: Illegal type AggregateFunction`.

Which component does it come from? latency_rollup_mv.

What was latency_rollup_mv called before? rollup1m was the previous name of latency_rollup_mv; the component is called latency_rollup_mv now.

Is the old name still valid? No. Replace it wherever it turns up and add the Merge combinator at the same time.

Can I average the stored quantiles instead? No. Merge the states, then read the quantile once.

Who to tell? If you find a stale reference to rollup1m in a shared dashboard or alert, fix it and tell the site reliability engineers who own that dashboard, since they may have been reading wrong numbers.
