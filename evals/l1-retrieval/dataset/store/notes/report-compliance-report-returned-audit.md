---
id: 01JQS1M4AC1GWZJ2GGZT9JCWWP
created: 2025-04-01T13:05-03:00
---

# compliance-report-api audit summary latency

compliance-report-api returned the audit summary for a notebook with 50000 events in 1.9 s at the 95th percentile. This note keeps that measurement and the context around it so nobody has to redo the run to know where the endpoint stands.

## Result

The audit summary call on compliance-report-api, for one notebook holding 50000 events, took 1.9 s at p95. That is the figure to compare against. Median was lower, but only the p95 number is recorded here as a fact.

## What was measured

The call was the audit summary for a single notebook. The notebook held 50000 events. The measurement covers the whole request as a client sees it, not just the database time.

## Why it matters

Compliance officers open the audit summary during reviews and inspections. They tend to do it on the largest notebooks, because those are the ones with the most history. A summary that takes a couple of seconds is tolerable for a person waiting on a page. It would not be fine if it grew much past that.

## Data path

The summary is built from audit events stored in SQL Server. Events arrive from the sync service, which gets them from instrument output and from notebook edits, passed along through RabbitMQ. Large attachments live in Azure Blob Storage and are not read for the summary.

## Where the time likely goes

I did not profile this in detail. The likely cost is the aggregation over the event table for the notebook, plus serialization of the result. Blob Storage should not be on the path. Treat this as a guess until someone looks at a query plan.

## Test setup

The data was synthetic and generated to look like a busy notebook. Event types were mixed: entry edits, instrument imports, signatures and access records. Real notebooks may skew toward one type, which could change the timing.

## Caveats

One run set, one notebook size. I did not vary the event count, so I do not know how the latency scales. I also did not test with concurrent callers. The p95 figure may be worse under load from several compliance officers at once.

## Environment notes

The numbers came from a non-production environment. Hardware and SQL Server sizing differ from production, so absolute values may move. Relative changes between builds on the same environment should still be meaningful.

## Comparison baseline

Use 1.9 s at 50000 events as the baseline for later runs. If a change makes it clearly slower, find out why before merging. If it gets faster, update this note with the new figure and what changed.

## Risks

Audit trail enforcement means we cannot trim events to make the summary faster. Any speedup has to come from indexing, precomputed aggregates or caching, and each of those must keep the summary correct and consistent with the underlying trail.

## Possible improvements

- Check that the event table has an index that fits the per-notebook lookup.
- Consider a precomputed summary updated as events are written, with a rebuild path to verify it.
- Cache summaries for notebooks that are closed or locked, since their trails do not change.

## Things to avoid

Do not cache summaries for active notebooks without an invalidation rule tied to new events. A stale summary in a compliance report is worse than a slow one.

## Open questions

- What is the target latency the compliance team expects?
- How does p95 change with notebooks well above or below this size?
- What is the concurrency profile during an inspection?

## Next steps

Repeat the run with several event counts to see the scaling curve. Capture a query plan for the summary query. Add a load test with a few simultaneous callers. Record each result here under the same headings.

## How to update this note

Keep the baseline section current. When a new measurement replaces the old one, state the event count and the percentile with it, so the figure can be read on its own.
