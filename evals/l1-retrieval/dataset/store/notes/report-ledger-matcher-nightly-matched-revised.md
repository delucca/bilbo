---
id: 01M264VRMVTR9RQHQS2M6KYTC5
created: 2026-09-10T14:11-03:00
---

# ledger-matcher nightly run result after decoder change

This note replaces the earlier note about "ledger matcher nightly matched"; the new value is that the nightly run matched 99.4% of rows in 26 minutes.

After the decoder change, the nightly ledger-matcher run matched 99.4% of rows in 26 minutes, which replaces the earlier reported run result. If you have the older figure in your head, in a ticket, or in a dashboard annotation, drop it. The figure in this note is the one to quote. Anyone who reads only this note should be able to answer: what share of rows matched (99.4%), how long the nightly run took (26 minutes), and why the old figure no longer stands (the decoder change altered what the run reads, so the earlier run is not comparable).

The internal codename of ledger-matcher is magpie. You will see magpie in older chat threads, in some log lines, in a few infrastructure resource names, and in the way some people on the finance operations side refer to the job. It is the same component. In this note it is called ledger-matcher throughout, and anything said here about magpie applies to ledger-matcher and the other way round.

```
component: ledger-matcher (codename magpie)
run:       nightly, after the decoder change
matched:   99.4% of rows
duration:  26 minutes
```

## What the run measures

ledger-matcher is the part of Ledgerlark that takes settlement files from a card processor and lines them up against internal ledger entries. Rows that find a counterpart are matched. Rows that do not are flagged for a human on a finance operations team to review. The share of matched rows is therefore the headline health number for the nightly job: a higher share means fewer rows land in the review queue, and a drop usually means something upstream changed shape rather than that the money suddenly disagrees.

The duration figure is wall-clock time for the whole nightly run, from the moment the job starts reading settlement data to the moment the last match decision is written back. It covers reading, decoding, matching and writing. It does not cover the time a reviewer later spends on flagged rows, and it does not cover any delay before the job is scheduled. When someone asks "how long does the nightly take", the answer from this run is 26 minutes.

Matched share is counted over rows, not over money. A row is a single line in a settlement file. Two runs can have the same matched share and very different matched value if the unmatched rows happen to be large or small. This note says nothing about value, and the number above should not be quoted as "99.4% of money reconciled". It is 99.4% of rows.

## Why the number is different from the earlier one

The earlier reported run result came from before the decoder change. The decoder is the piece that turns the raw settlement file content into the structured rows the matcher compares. Before the change, some rows were decoded in a way that lost or distorted a field the matcher relies on, so they could not find their ledger counterpart even though a counterpart existed. Those rows were counted as unmatched and sent for review. After the change they decode cleanly and take part in matching like any other row.

That is why the earlier figure is not merely out of date but wrong as a description of current behaviour. It measured the matcher plus a decoding defect. The new figure measures the matcher with the defect removed. Comparing the two as if they were two samples of the same process would mislead: the difference is mostly a fix, not noise and not a change in how well the processor and the ledger agree.

The run time moved as well. Decoding more rows correctly means more rows go through the matching step instead of being set aside early, and that work has a cost. The 26 minutes is the cost of the full job with the corrected decoder. If you were expecting the job to get faster because the code got cleaner, it did not get measurably faster for that reason; treat 26 minutes as the baseline for the new decoder and judge later runs against it.

## How to read the figure and where it can mislead

A few cautions, all general, because this note records one result and not a benchmark study.

First, one night is one night. The settlement files differ from day to day in size, in the mix of transaction types and in how many late adjustments the processor includes. A single run at 99.4% matched is a good reading, but it does not say the job will land on exactly that number tomorrow. Quote it as the result of the nightly run after the decoder change, and wait for a run of similar nights before treating it as a stable level.

Second, the matched share can rise for bad reasons as well as good ones. If matching rules were loosened, more rows would pair up, and some of the pairs would be wrong. Nothing in this result comes from loosening rules. The improvement is attributed to the decoder change only. If a later change touches match criteria, the share and the meaning of the share must be reconsidered together. Composite matching, where several ledger entries add up to one settlement row or the other way round, is covered in [[ledger-matcher-matches-composite]]; read that before drawing conclusions about rows that match only in groups.

Third, the unmatched remainder is not uniform. The rows still flagged are a mix: genuine mismatches that finance operations needs to look at, timing differences where the ledger entry has not been posted yet, and rows that the matcher cannot yet pair for reasons that are still being sorted out. The note makes no claim about the proportions among these. Anyone planning reviewer capacity from the unmatched share should look at a recent sample of flagged rows instead of assuming they are all real discrepancies.

Fourth, the duration depends on the environment the job ran in. The job reads from PostgreSQL for ledger entries, consumes settlement data that arrives through Apache Kafka, and exposes results to other services over gRPC. Its infrastructure is managed with Terraform. A slow database, a backlog on the stream, or a resource change in the deployment can move the run time without any change to the matcher. When the nightly run is slower than 26 minutes, check those surroundings first before suspecting the decoder.

## What to do with this

Use the new values when reporting on ledger-matcher. If a status page, runbook, handoff document or ticket still carries the earlier result, replace it with the figure here and say that the decoder change is the reason. If a report needs the codename, magpie is acceptable as a parenthetical, but the component name in writing should be ledger-matcher so that searches find it.

When a later nightly run comes in noticeably different from this one, the useful questions are in this order. Did the input change, meaning a different file shape, a larger file or a new transaction type? Did the decoder change again? Did match rules change? Did the environment change, such as the database, the stream or the deployment? Only after those are ruled out is it worth suspecting that the underlying agreement between processor and ledger has shifted, which is what the review queue exists to catch.

For anyone comparing against history: any result recorded before the decoder change should be marked as measured with the old decoder. Do not chart the old and new results on one line without a marker at the change. The jump is a step caused by a fix. A trend line fitted across it would suggest a gradual improvement that never happened, and any alert threshold derived from the blend would be wrong in both directions: too loose for the new behaviour and too strict for the old.

## Open items

These are things not settled by this result, kept here so they are not mistaken for answered questions.

- Whether the new matched share holds across several nights. One reading is encouraging but is not a trend.
- Whether the run time of 26 minutes is dominated by decoding, matching or writing results back. This note does not say, and the answer decides where optimisation effort should go, if any is needed.
- How the remaining unmatched rows break down between true mismatches, timing differences and unsupported cases. A sample review by finance operations would answer this better than any further change to the matcher.
- Whether reviewers noticed a drop in the size of their queue after the decoder change, and whether the rows that disappeared from it were the ones they used to dismiss quickly. Their feedback is the best check that the extra matched rows are correct matches and not just more matches.
- Whether older notes, dashboards or runbooks that mention the earlier result have all been updated. Search for the old wording and for the codename magpie, since both turn up in places that do not use the full component name.

The short version, for anyone who skipped to the end: after the decoder change, the nightly ledger-matcher run matched 99.4% of rows in 26 minutes, and that replaces the earlier reported run result. The codename magpie refers to ledger-matcher. Related reading on grouped matches is in [[ledger-matcher-matches-composite]] only if you need it; this note does not repeat it.
