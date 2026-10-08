---
id: 01KZ6WPMYXZN7RZFH66HX0Z145
created: 2026-08-04T14:21-03:00
---

# ledger-matcher nightly run: match rate and duration

The nightly ledger-matcher run matched 98.7% of 2.4 million settlement rows and finished in 41 minutes. This note records that result, what we think it says about the component, and what to look at next. It is a report on one run, not a spec. Treat the figures as a baseline to compare later runs against, not as a target.

```
run: nightly ledger-matcher
settlement_rows: 2.4 million
matched: 98.7%
duration: 41 minutes
```

## What the run did

ledger-matcher reads card-processor settlement files, loads the rows, and tries to pair each row with an internal ledger entry held in PostgreSQL. Rows that pair cleanly are marked matched. Rows that do not pair are flagged for review, and finance operations people at the marketplace pick them up the next morning. The nightly run is the normal path. It processes the whole batch of settlement rows that arrived since the previous run.

This time the batch held 2.4 million settlement rows. The run matched 98.7% of them and finished in 41 minutes. The remainder, the rows that did not match, went to the review queue. We did not see an abnormal failure: the process exited normally, the job did not need a restart, and no partial batch was left behind. The Kafka consumers that carry match events to downstream services drained without a visible backlog by the time the run ended.

The percentage is computed over settlement rows, not over ledger entries. That distinction matters. A single ledger entry can legitimately correspond to several settlement rows, for example when a processor splits a payout or reports a fee separately, and a settlement row can occasionally have no ledger counterpart at all because the marketplace never booked the underlying sale. So the matched share is a statement about the settlement side only. It says nothing directly about how many ledger entries were left without a settlement partner. If someone needs that figure, it has to be computed separately from the ledger side.

## How to read the 98.7% figure

A match rate in the high nineties looks good, but the remaining share is not small in absolute terms. With 2.4 million rows in the batch, the unmatched share is still a large pile of rows for a human team to look at. Finance operations do not review rows one at a time in practice; they work in groups that share a cause. What matters for them is whether the unmatched rows cluster into a few explainable groups or are scattered with no pattern. Clustered mismatches are cheap to clear. Scattered ones are expensive.

From what we can see, the unmatched rows lean toward a few familiar causes, listed here roughly in order of how often they show up in review:

- Timing differences. The processor settles a transaction on a different day than the one the ledger booked it, so the pair exists but falls in different batches. These usually resolve themselves in a later run, which means the unmatched share on any one night overstates the permanent mismatches.
- Amount differences. Fees, currency conversion rounding, or partial refunds make the settlement amount differ from the ledger amount by a small margin. Whether such rows count as a match depends on the tolerance the matcher is configured with. Changing that tolerance moves the headline percentage without changing the underlying books, so be careful when comparing runs made under different settings.
- Missing references. The settlement row carries no usable reference to the internal transaction, or the reference is malformed. The matcher cannot pair these by key and falls back to weaker signals, which sometimes fail.
- True orphans. Money moved at the processor with nothing booked on our side, or the reverse. These are the rows finance cares about most, and they are a small part of the unmatched set.

Because timing differences clear on their own, the number to watch over time is not the single-night rate but the share of rows that are still unmatched after they have had a chance to settle. We do not yet track that as a separate metric. It would be a more honest measure of matcher quality than the nightly percentage.

## Where the 41 minutes go

The run finished in 41 minutes for 2.4 million rows. We have not profiled this run in detail, so what follows is our working picture of the time, not a measurement. Treat it as a list of places to look first when the duration moves.

Ingestion comes first. Settlement files arrive from several processors in different formats, and each is parsed and normalized into a common row shape before matching. Parsing is Go code and is rarely the slow part. The cost sits in writing the normalized rows into PostgreSQL, which is bulk work and sensitive to how the target tables are indexed and how much autovacuum activity is going on at the time.

Matching is next. The matcher joins normalized settlement rows against ledger entries first by strong keys and then by weaker ones. The strong-key pass takes most of the rows and is quick. The weaker passes handle the leftovers and cost more per row, because they compare on amount, date window, and counterparty rather than a direct key. The fewer rows that fall through to those passes, the faster the whole run. This is one reason the match rate and the duration are linked: a night with more awkward rows is both lower on match percentage and longer on the clock.

Last is publishing results. Match outcomes and review flags are emitted as events on Apache Kafka, and the review queue is fed from them. If a consumer is slow, the run itself can still finish while the downstream work lags. So the finish time of the run does not guarantee that reviewers see everything straight away. When checking a night, look at both the run end and the point at which the review queue stopped growing.

Other services that need match data call ledger-matcher over gRPC rather than reading its tables directly. That traffic is light during the nightly window, so we do not think it competes with the run, but we have not ruled out contention on the database during busy periods.

## Operational notes

The infrastructure around ledger-matcher is described in Terraform, including the database, the Kafka topics, and the compute the run executes on. If a future run is much slower, check whether compute or database sizing changed in a recent apply before blaming the matching logic. A sizing change is the quickest explanation for a sudden shift in duration and the easiest to rule in or out.

A few habits that have helped when reading a run:

1. Compare like with like. Only compare nights with similar row volume and the same matcher settings. A quiet night and a heavy night will differ in duration for reasons that have nothing to do with code.
2. Separate the rate from the volume. A drop in the matched percentage on a night with a large share of new processor formats or a processor outage upstream is usually a data problem, not a matcher regression.
3. Look at the unmatched set by cause before alarming anyone. A rise driven by timing differences is mostly harmless; a rise driven by missing references points at an upstream change.
4. Keep the baseline in this note current. When a run is clearly representative, update the figures here instead of starting another note.

If a run fails midway, the safe move is to rerun the whole batch rather than resume by hand. The matcher is meant to be idempotent over a batch, so a rerun should produce the same pairs and not duplicate review flags. We have relied on that, but if you change how flags are written, recheck it, because duplicate flags are the kind of fault that reviewers notice first and trust the system less for.

## What to do next

The 98.7% over 2.4 million rows in 41 minutes is a reasonable baseline, and nothing in it demands action today. The open work is mostly about making the number more informative rather than higher.

First, add a measure of rows still unmatched after the settlement window has passed, so the headline figure stops mixing timing noise with real mismatches. Second, record the matched share from the ledger side as well, so we can see ledger entries that never found a settlement partner. Third, break the 41 minutes into ingestion, matching and publishing, so that a change in duration can be pinned on a stage right away. A simple timing log per stage would be enough and would not need new infrastructure.

Fourth, write down the amount tolerance the matcher uses and keep it next to the baseline figures. Without it, a later run with a better percentage might simply have a looser tolerance, and nobody could tell. Fifth, talk to the finance operations team about which unmatched groups cost them the most time. Their answer may show that a small improvement in one weak pass is worth more than a large improvement in overall rate.

Until those are in place, use this run as the reference point. If a later night differs a lot in either the matched share or the duration, first check volume, matcher settings, and recent infrastructure changes, in that order, before digging into the matching code itself.
