---
id: 01M0YP3FMFWWZ3DTWJE4JK0AMY
created: 2026-08-26T06:23-03:00
---

# analyst-orders-api spec: latency target and behavior

This is the working spec for analyst-orders-api, the service merchandising analysts use to look at, review and adjust the replenishment orders that ShelfSense generates. It is written quickly and kept short on detail on purpose. Where a number or a name is not given here, it is not settled, so check with whoever owns the service before assuming anything. The one hard performance requirement is that the order listing endpoint must keep p95 latency under 300 ms. Everything else in this note either explains why that target exists, what it constrains, or what the service does around it.

## Purpose and users

ShelfSense predicts store-level stockouts and turns those predictions into replenishment orders. The prediction work happens in batch, in Spark jobs that read and write Delta Lake tables and are scheduled by Airflow. Aggregated and reporting-friendly data also lands in Snowflake. None of that is interactive. The analyst-orders-api is the interactive part: a thin service in front of the order data so that a merchandising analyst at a grocery chain can open a screen, see what the system proposes to order for their stores, and act on it before the order goes out.

The typical analyst is working through a list. They open the order listing, filter to a region, a category or a set of stores, scan for orders that look wrong, open a few, adjust quantities or hold them, and move on. They do this many times in a session. That usage pattern is why latency matters so much for the listing endpoint specifically. A slow list makes the whole tool feel broken, because the analyst pays the delay again on every filter change and every page. A slow detail view is annoying but it happens once per order they care about.

The service is not a general data access layer. It does not serve forecasts, model diagnostics or raw sales history. Those belong to other tools and to the Snowflake side. If someone asks for a new endpoint that returns bulk historical data, the answer is usually that it belongs somewhere else, because bulk reads are exactly what threatens the latency target below.

## Latency requirement

The analyst-orders-api must keep p95 latency under 300 ms for the order listing endpoint. That is the requirement, stated once so nobody has to guess. The measure is the ninety-fifth percentile of response time for listing requests, taken as the server sees it from receiving the request to finishing the response, over a rolling window that the monitoring setup defines. A reader of this note alone should come away knowing three things: the endpoint in question is the order listing, the statistic is the ninety-fifth percentile and not the average, and the ceiling is 300 ms.

Why the percentile and not the mean: the mean hides the slow requests that analysts actually remember. A listing that is fast most of the time and very slow for large chains feels unreliable. Holding the high percentile down forces us to care about the heavy filters, the wide date ranges and the stores with large order counts, which are the cases that go wrong first.

Why this endpoint and not all of them: the listing is the entry point to everything else and is hit far more often than any other call. Other endpoints are expected to be reasonable but have no committed target in this spec. If a different endpoint gets a target later, add it here as its own statement rather than loosening this one.

What counts against the budget: query time against the backing store, any permission checks, serialization, and any work done to build pagination. What does not count: network time between the analyst's browser and our edge, and time the client spends rendering. If the budget is blown, the first suspects are the query and the serialization, in that order, because those are the parts we control and the parts that grow with data volume.

What the target rules out in practice:

- Computing aggregates on demand by scanning large tables. Anything an analyst sees in a list row must already be materialized or cheap to derive.
- Calling out to the batch systems synchronously. The service must never wait on a Spark job or an Airflow run to answer a listing.
- Unbounded result sets. Every listing is paginated, and the page size has a ceiling that the service enforces whatever the client asks for.
- Per-row lookups that fan out into many small queries. Joins or batched lookups only.

If a change would plausibly push the percentile over the ceiling, it needs a measurement before merge, not after. A feature that is correct but pushes the listing over its budget is not done.

## Data sources and freshness

The orders the service shows are produced by the batch pipeline. Spark jobs written in Scala compute the proposed orders and write them to Delta Lake tables. Airflow schedules and orders those jobs. For the interactive path, the service reads from a serving copy of that data rather than from the large analytical tables directly. The reason is the latency target: the analytical tables are shaped for batch work and wide scans, and the serving copy is shaped for the filters analysts actually use.

Snowflake is where reporting and cross-chain rollups live. The analyst-orders-api should not use it for the listing path. Round trips to a warehouse are variable and are not a good fit for a fixed ceiling on tail latency. If a screen needs a warehouse-derived figure, that figure should be pushed into the serving copy by the pipeline ahead of time, not fetched at request time.

Freshness is a tradeoff the analysts understand. A listing reflects the most recent completed pipeline run, not the live state of the store. The response should carry enough information, such as when the underlying data was produced, for the analyst to tell how stale it is. We would rather show slightly old data quickly than make the analyst wait for fresh data. When a pipeline run is late or failed, the service keeps serving the last good data and says so, instead of erroring.

Edits made by analysts, such as a changed quantity or a hold, are written by the service to its own store of overrides. They are applied on top of the pipeline output when the listing is built. The next pipeline run must respect those overrides and must not silently erase them. How exactly the pipeline reads them back is owned by the batch side, and the contract between the two should be written down on that side too.

## Endpoint behavior

The order listing returns a page of proposed and in-progress orders for the stores the caller is allowed to see. It accepts filters for store, region, category, status and date window, plus sorting and a cursor or page token. The shape of each row is kept deliberately small: enough to scan and decide whether to open the order, not the full detail. Putting the heavy fields in the listing is the easiest way to lose the latency budget, so resist requests to add them. If the analyst needs the heavy field, it goes in the detail view.

The order detail endpoint returns one order with its lines, the quantities the system proposed, any analyst overrides, and the reasons shown for the proposal. It is allowed to be slower than the listing, but it should still be quick enough that opening an order does not feel like a page load.

The update endpoints let an analyst change quantities, place a hold, release a hold or approve an order. They validate against the rules for the chain, for example pack sizes and minimum or maximum quantities, and they reject invalid edits with a clear message that names the line and the rule. They are idempotent where possible, so a retry after a flaky connection does not double-apply a change.

Sorting and filtering rules worth remembering:

- Default sort is stable and deterministic, so paging never repeats or skips a row when two orders tie.
- Filters are combined with AND. Multi-value filters on a single field are combined with OR within that field.
- Unknown filter names are rejected, not ignored, so a typo does not return an unfiltered list that looks plausible.
- Counts shown alongside a page may be approximate for very large result sets. Exact totals are not promised, because computing them is expensive and threatens the budget.

Numbers that analysts see in the service, such as quantities and percentages, follow the same rounding conventions as the rest of the product. Those conventions are written up in [[shelf-metrics-must-round-revised]], and the service should follow that note rather than restate or reinvent the rules here. If the two ever disagree, that note wins and this one should be corrected.

## Operating and measuring the target

The latency target is only useful if it is measured the same way every time. The service emits request timing for each endpoint, tagged by endpoint name, and the dashboards compute the ninety-fifth percentile for the listing from that. The alert is on the listing percentile crossing the ceiling for a sustained period, not on a single slow request. A one-off slow request is noise; a sustained drift is a regression.

When the alert fires, the order of investigation is roughly this. First check whether a recent deploy changed the listing query or the serialization. Then check whether the serving copy is healthy: a stale or oversized copy, a missing index-like structure, or a compaction that has fallen behind will all show up as slower reads. Then look at the traffic mix, because a change in which chains or filters are heavily used can move the percentile without any code change. Only after those should anyone consider scaling the service out, since adding instances does not help if the query is the bottleneck.

Load testing for the listing should use a realistic mix and not just the easy case. The realistic mix includes the largest chains, wide date windows, several filters at once, and deep pages. A test that only exercises a small chain with no filters will pass comfortably and prove nothing. Keep the test data shaped like production: similar store counts, similar order counts per store, similar skew.

Before any change that touches the listing path, run the load test and compare the percentile with the previous run. Record the result in the change description. If the percentile moved noticeably toward the ceiling, even without crossing it, that is worth a conversation, because the margin is what absorbs growth in data and traffic.

## Failure handling

When the serving copy is unavailable, the service should fail fast with a clear error and not hang until a timeout. A fast failure keeps the percentile honest and lets the client show a useful message. The service should never fall back to scanning the analytical tables or calling the warehouse as a backup, because that fallback is slow, expensive and can take down the very system it is trying to rescue.

When a pipeline run is late, the listing keeps serving the last good data and marks it as old. When the pipeline produced bad data, the fix belongs on the batch side, but the service should have a way to pin to an earlier good version of the serving copy so analysts are not stuck looking at nonsense while the pipeline is repaired. Pinning is an operator action and should be logged.

Override writes that fail must tell the analyst plainly that the change was not saved. Silent loss of an edit is worse than any slowness. If a write times out and its outcome is unknown, the client is told it is unknown and can reread the order before retrying, which is another reason the update endpoints are idempotent.

Permission failures return a distinct error from not-found, but the message should not reveal whether a store the caller cannot see exists. Rate limiting, if it is needed, applies per caller and is tuned so that ordinary analyst use never hits it. Its purpose is to protect the percentile from a runaway script, not to ration normal work.

## Open questions and things to watch

A few items are not settled and should not be treated as decided just because they appear here.

- Whether the listing should offer exact totals as an opt-in, and what that would cost the budget. Current stance is no, until someone shows a measurement.
- How the serving copy is refreshed as the pipeline finishes: all at once, or store by store. A partial refresh could show mixed freshness in one list, which analysts may find confusing.
- Whether other endpoints deserve their own latency targets. If so, add them here as separate statements.
- How growth in the number of chains will affect the percentile. The margin under the ceiling should be watched over time, not only checked at release.
- Whether the override store and the serving copy should be one system or two. Two is simpler to reason about today but makes the merge on read a cost inside the budget.

The one thing that is not open: the order listing endpoint of analyst-orders-api keeps p95 latency under 300 ms. Design choices that conflict with that should change, not the target, unless the owners agree to revise it and this note is updated to say so.
