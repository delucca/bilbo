---
id: 01KSZ48RB2S2PYP2WS9REG7706
created: 2026-05-31T10:41-03:00
---

# spans_raw queries without a start_time filter fail with Code: 158

Any query on spans_raw that has no start_time filter scans every partition of the table. On our setup that ends in a row-limit error, and the text is: `Code: 158. DB::Exception: Limit for rows (including read) exceeded`. The error does not mean the table is broken, the cluster is unhealthy, or the data is corrupt. It means the query, as written, asked ClickHouse to read more rows than the profile of the calling user allows, because nothing told the engine it could skip old partitions. Add a start_time bound and the same query usually runs fine.

The table also goes by a short name. rawtbl is short for spans_raw. You will see rawtbl in scratch SQL, in a few dashboard variables, in chat messages and in older notes. It is the same table. When you search the repo, the runbooks or Grafana for places that touch the span storage, search for both spellings, or you will miss half of them. In this note I use spans_raw everywhere except where I am explaining the alias.

This is the first thing to check whenever someone says a TraceQuill query died with a limit error, or a post-deploy latency summary came back empty or failed. Nine times out of ten the cause is a missing time filter, and the fix is a one-line change to the query.

## What the failure looks like

The error text is the one above, with the code and the exception class. It arrives from ClickHouse as a server-side exception, so every client shows it a little differently.

In the ClickHouse command-line client you see the exception printed after the query has already run for a while. The delay is the useful hint: the server did real work reading data before it hit the limit. A query that is rejected immediately for some other reason, such as a syntax error or an unknown column, comes back at once. A limit error that arrives after a long wait is almost always a full scan.

In Java code that goes through the JDBC driver, the error shows up as a SQL exception whose message contains the same text. Depending on the wrapper, the cause chain can be several layers deep, and the useful line is at the bottom. If you only log the top-level message, you may see a generic data-access failure and not realize it is this problem. When you triage a stack trace from the summarizer or from the API layer, scroll to the innermost cause and look for the code and the phrase about rows.

In Grafana, a panel backed by a ClickHouse data source shows the error in the panel's red corner marker. Hover over it and the message is there. If the panel is a table or a time series with a query that uses a Grafana time macro, the filter is normally applied for you and the panel will not fail this way. The panels that fail are the ones where somebody wrote a raw query that does not reference the dashboard time range, or where the time macro was applied to a different column than start_time. That second case is subtle. A filter on some other timestamp column does not help the partition pruning, so the scan is still full.

In the post-deploy regression summary job, the symptom is different again. The job runs after each deploy, reads spans for the window before and after the rollout, and compares latency. If a query inside it lacks the start_time bound, the job fails with this error, and the summary for that deploy is missing or marked failed. The site reliability engineers then see a deploy with no summary and may assume the deploy was clean. It was not necessarily clean; it was just not analyzed. Treat a missing summary as unknown, not as good.

## Why it happens

spans_raw holds every span the collectors receive, as written by the OpenTelemetry pipeline. It is the largest table in the system by a wide margin. It is partitioned by time, and the partition key is derived from start_time. The primary ordering also leads with time-related columns, so a filter on start_time does two things at once: it lets ClickHouse drop whole partitions without opening them, and it lets the sparse primary index skip granules inside the partitions that remain.

Without a start_time condition, neither of those mechanisms can run. The planner has to consider every partition, and the engine starts reading. The query profile used by the services applies a cap on rows read. When the count crosses the cap, the server aborts and returns the exception with code 158. The cap exists on purpose. Without it, one careless ad hoc query from a laptop could pull so much data through the cluster that ingestion and the dashboards slow down for everyone. So the error is a safety net working as designed, and raising the cap is not the right response.

A few things make people hit this by accident.

First, filters on other columns look selective but do not prune partitions. A filter on a trace identifier feels narrow, since it picks out one trace. But the table is not ordered by trace identifier first, so ClickHouse cannot jump to it. It still has to look at data across all partitions, and the row cap trips before the filter has narrowed anything. The same goes for filters on service name, span name, status, or an attribute value. These are good additional filters. They are not substitutes for the time bound.

Second, the time filter can be present but ineffective. If the condition wraps start_time in a function, for instance converting the column to a different type, a date string, or a time zone, the planner may not be able to use it for pruning. Keep the column bare on one side of the comparison and put the conversion on the constant side. Comparing the column against a value of the right type is better than comparing a converted column against a string.

Third, joins and subqueries. If a query joins spans_raw to another table, or reads it inside a subquery, the filter has to be on the spans_raw side. A time filter on the outer query or on the other table does not push down reliably. When in doubt, write the bound directly inside the subquery that reads spans_raw.

Fourth, views and materialized views built on spans_raw. A view that selects from spans_raw without a time condition passes the problem on to whoever queries the view. If a view's definition has no bound, the caller must supply one in a form that the planner can push through. Check the definition before you assume that querying a view is safe.

Fifth, count queries. People run a quick row count on the table to see how much data there is, or to check ingestion. A bare count over spans_raw is a full scan from the planner's point of view and fails the same way. Count over a recent time window instead.

## How to query spans_raw safely

The rule is short: always bound start_time, and bound it as tightly as the question allows.

Write the time condition first when you draft a query, before the select list is finished. It is easy to forget it once the interesting part is written. Make it a habit and reviewers will start to expect it.

Choose the window from the question. For a post-deploy comparison, the window is the period before the rollout and the period after it, and nothing else. Do not widen it to be safe; wider windows read more, run slower and make the row cap more likely to trip even with the filter present. If you need a baseline, take a bounded baseline window and not the whole history.

Keep the bound on the raw column. Compare start_time to a constant or to an expression of the current time minus an interval. Do not apply functions to the column.

Add the narrower filters after the time bound, not instead of it. Service name, operation name, and status narrow the work inside the partitions that survive. Put them in; they help.

When you explore interactively, start with a very short window, look at the shape of the data, and then widen step by step. If a window that looks small still fails with the limit error, the table has a lot of traffic in that period, and you should narrow it further or add selective filters, rather than ask for the cap to be changed.

When you write Java that builds queries, make the time bound a required parameter of the query-building method. If the method signature takes a start and an end, nobody can forget them. Do not offer an overload that omits them. If a caller really needs an unbounded read, that is a design discussion, not a convenience method. Reviewers should push back on any code path that builds a select on spans_raw without the time parameters.

When you write Grafana panels, use the dashboard time range macro on start_time, and check by looking at the final query in the query inspector that the condition really refers to that column. If the dashboard has a variable that holds the table name, and the variable's value is rawtbl, the same rule applies: the alias is the same table and the same cap. People sometimes assume rawtbl is a smaller or sampled copy because of the shorter name. It is not. It is spans_raw under a second name.

When you review someone else's query, ask three things. Is there a start_time bound? Is the column bare in that condition? Is the bound inside the part of the query that actually reads spans_raw? If all three are yes, the limit error is unlikely.

## The rawtbl alias

rawtbl is short for spans_raw. The short form shows up in a few places for historical reasons: early prototypes used it, and it stuck in some scratch queries, saved Grafana variables, and shell history. It is not a different table, it is not a view with different retention, and it does not have a different cap. Anything true of spans_raw is true of rawtbl.

The practical consequences are these.

When you grep for usages, grep for both names. A search for only one of them gives a partial picture, and you may conclude that nothing in a given dashboard or job reads the table when something does.

When you read an incident thread or a chat log and someone says the rawtbl query failed with a rows limit error, that is this gotcha. Go straight to checking the time bound.

When you write new code or new docs, prefer the full name spans_raw. The short name is fine in conversation, but a reader who has never seen it will not know what it refers to, and a search for the full name will miss the file. If you edit a place that uses the short name and it is cheap to change, change it to the full name, and mention the old spelling in a comment or in the commit message so that people searching for it can still find the change.

When an alert or a runbook step says to check rawtbl, the step means the span table. Do not go hunting for a separate object.

## Diagnosing a failure quickly

When a report of Code 158 comes in, work through this list in order. It is quick, and it catches nearly every case.

First, read the exact error text. Confirm it matches the message at the top of this note: the code, the exception class and the phrase about the limit for rows including rows read. Other limit errors exist, such as ones about bytes, memory, execution time or result size, and each has its own cause. This note covers the row-limit case. If the text differs, do not apply the advice here blindly.

Second, find the query that failed. In the client it is the statement you just ran. In a service it is in the logs or in the server-side query log, which records the text of the statement, the user, the duration and the exception. Look at the statement for the time condition.

Third, check that the time condition is on start_time, not on some other column, and that the column is not wrapped in a function. If either is wrong, fix the query.

Fourth, check that the bound is narrow enough. If the window is very wide, narrow it. If the question really needs a wide window, split it into several smaller windows, run them one after another or in parallel with care, and combine the results. Aggregates that can be merged, such as counts and sums, combine easily. Percentiles do not combine by averaging, so for latency percentiles prefer an aggregate state that can be merged, or keep the window small enough to compute directly. Do not average percentiles across windows and call it a percentile; the result is wrong in a way that looks plausible, and plausible wrong latency numbers are the worst outcome for a tool whose job is to flag regressions.

Fifth, check whether a view or a macro is hiding the table. If the failing query selects from a view, open the view definition and look for the same problems inside it.

Sixth, check whether the failure is new. If the same query worked yesterday and fails today, something changed. Possible causes are that a Grafana variable now resolves to something empty, so the time macro expands to no condition; that someone edited a saved query; that a deploy changed the Java code which builds the statement; or that the cap in the user profile was lowered. Compare against the previous version of whatever changed.

Seventh, only after all of that, consider that the table really has grown so that even a tight window now exceeds the cap. That is rare, but it can happen after a burst of traffic, for example after a change in sampling, or when a service starts emitting many more spans per request. In that case the answer is a tighter window, more selective filters, or pre-aggregation, and not a higher cap.

## What not to do

Do not raise the row cap in the user profile to make the error go away. It hides the full-scan problem, makes the query slow, and puts the cluster at risk. The next person with a similar query will have no safety net. If a particular job needs a different limit for a legitimate reason, that is a reviewed change with an owner and a reason written down, and it still needs a time bound.

Do not work around the error by adding a trivial always-true time condition. A condition that the planner cannot use for pruning gives the appearance of a filter and none of the benefit. The error will come back, and the code will look as if it were handled.

Do not retry the same query in a loop. The failure is deterministic. It reads the same data and trips the same cap every time, and each retry wastes cluster capacity. Services should treat this error as non-retryable and surface it with the query text so a human can fix it.

Do not copy the query into a dashboard without adapting the time filter to the dashboard range. A query that works in the client with a hand-written bound can fail in Grafana if the bound is hard-coded to a window that no longer exists, or can silently return stale data. Use the dashboard's own time range.

Do not treat a missing post-deploy summary as a pass. If the summary job failed for this reason, fix the query and rerun the summary for that deploy, then read it. The regression you are looking for may be sitting in the window nobody analyzed.

## Notes for people maintaining the code

The cleanest guard is in the Java layer that builds statements. Make the span-reading methods take explicit start and end values, validate that the end is after the start, and reject windows beyond a configured maximum before the statement is sent. A clear application-level error, saying that the window is too wide or missing, is much easier to act on than a server exception that arrives after a long scan. Keep the server-side cap as the second line of defense, not the first.

Add a test that builds each query the service can emit and asserts that the generated text contains a condition on start_time. It is a cheap string-level check and catches the regression where somebody adds a new query and forgets the bound. If the code uses a query builder, assert on the builder's structure rather than the final string where that is possible.

In Grafana, keep a short note on the dashboards that use raw queries, saying that every query must reference the dashboard time range on start_time. Provisioned dashboards live in the repo, so reviewers can check the JSON for panels that query the table directly and lack the macro. A quick scan of the provisioning files for the table name, in both spellings, followed by a look at each hit, is a reasonable review step when dashboards change.

For Kubernetes-side jobs, such as the one that runs the post-deploy summary, make sure the job passes the deploy timestamps into the query as parameters. If the job reads the rollout time from the cluster and the read fails or returns nothing, the code should stop with a clear message and not fall through to a query with an empty or default window. A default window of the whole table is exactly how this error gets triggered from automation. Prefer failing early with a specific message over sending a statement with no usable bound.

For OpenTelemetry-side changes, remember that anything that increases span volume makes the tolerance for wide windows smaller. If the collectors are reconfigured to keep more spans, or an instrumented service becomes chattier, windows that used to fit under the cap may stop fitting. The first sign may be a Code 158 on a query that has always had a time bound. When that happens, check recent changes to sampling and instrumentation before blaming the query.

## Quick reference

Symptom: the server returns the row-limit exception quoted at the top, after some delay.

Cause: a query on spans_raw, also called rawtbl, with no effective start_time filter, so every partition is considered and the read passes the cap.

Fix: add a bounded condition on the bare start_time column, as tight as the question allows, inside the part of the query that reads the table. Keep other filters as extras.

Avoid: raising the cap, retrying, wrapping start_time in functions, putting the bound only on the outer query, treating a failed summary as a clean deploy, averaging percentiles across windows.

Alias: rawtbl is short for spans_raw. Search for both names. Prefer spans_raw in new writing.

If the message differs from the one quoted here, this note does not apply. Look at the other limits in the user profile and the query log before deciding what is wrong.
