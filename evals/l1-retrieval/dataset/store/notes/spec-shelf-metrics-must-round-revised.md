---
id: 01KZDPTSVQAQS8T86EDM2JWVN9
created: 2026-08-07T05:53-03:00
---

# shelf-metrics-lib spec: fill_rate rounding

This note replaces the earlier note about "shelf metrics must round". The new value is that shelf-metrics-lib rounds `fill_rate` results to 3 decimal places, replacing the earlier precision, so the numbers match the analyst dashboard.

The rest of this note is the working spec for that behavior: what is rounded, where, what is not, and how to check it. It is written quickly, so treat the Open questions section as the list of things nobody has confirmed.

## Summary of the rule

Every `fill_rate` value that shelf-metrics-lib returns is rounded to 3 decimal places. The rounding happens inside the library, at the point where the metric is finalized. Callers do not round again and should not need to. The reason is the analyst dashboard: merchandising analysts compare what the dashboard shows with what ShelfSense reports in replenishment views and exports, and the two used to disagree in the trailing digits. The dashboard shows 3 decimal places, so the library now matches it.

The earlier precision is gone. Nothing should depend on it. If you find code that does, that code is stale and needs to change, not the rule.

## Why this change was made

Analysts reported that a store's `fill_rate` looked different in the dashboard than in a downstream file. The difference was only in the last digits, but it caused repeated questions and some distrust of the numbers. The dashboard was the reference because analysts look at it first. Rather than teach the dashboard to show more digits, the decision was to bring the library down to what the dashboard displays.

This is a presentation-driven choice. The extra digits carried no meaning the analysts could use, because fill rate is derived from counts of units and these are not precise enough to justify more digits at the store level.

## Scope of the rule

The rule covers `fill_rate` as produced by shelf-metrics-lib. It applies wherever the library is used: Spark jobs written in Scala, scheduled runs orchestrated by Airflow, and any code that reads the result and writes it to Delta Lake or Snowflake. If a value reaches those systems through the library, it is already rounded.

The rule does not by itself change other metrics. If another metric needs the same treatment, that is a separate decision and should get its own line in this spec. Do not assume a blanket rule for the whole library.

## What is rounded and what is not

Rounded: the final `fill_rate` value for each entity the library reports on, such as a store, a store and product pairing, or a group of those, depending on the grain of the call.

Not rounded: the numerator and denominator that go into `fill_rate`, and any intermediate columns used to build it. Those stay at full precision so that re-aggregating them gives correct results. Rounding is the last step, never an input to later arithmetic.

This matters for rollups. A rolled-up `fill_rate` must be computed from the unrounded counts, then rounded once. Averaging already-rounded store values gives a different and worse answer, and it is the most likely way to introduce a bug here.

## Rounding mode

The intent is ordinary rounding to the nearest value at the stated precision, using the same convention the dashboard uses. Ties should resolve the same way in both places. If you change the mode in the library, check the dashboard first, because the whole point is that the two agree.

I have not confirmed which tie-breaking convention the dashboard uses at the exact midpoint. Ties are rare with real data, but they will appear in tests built from small round numbers. See Open questions.

## Where rounding sits in the pipeline

In a typical run, Airflow schedules a Spark job, the job reads stock and sales data from Delta Lake, shelf-metrics-lib computes the metrics, and results are written out, including to Snowflake for the dashboard. Rounding happens inside the library step, before results are written. That means Delta Lake tables that store library output hold rounded `fill_rate` values, and Snowflake receives the same rounded values with no further transformation.

If some job recomputes `fill_rate` outside the library, it is not covered and will not match. Prefer calling the library over re-implementing the formula.

## Interaction with Delta Lake

Tables in Delta Lake that already hold historical `fill_rate` values were written under the earlier precision. They are not rewritten by this change. So a table can contain old values at the earlier precision and new values at 3 decimal places side by side, split by run date.

When comparing across the change date, either round the old values at read time or expect small differences. Do not treat a tiny difference across that boundary as a regression. If a backfill is wanted, it should be done deliberately and recorded in its own note.

## Interaction with Snowflake and the dashboard

The dashboard reads from Snowflake. After the change, the value stored there for `fill_rate` is already at 3 decimal places, so the dashboard displays what is stored. Check that the column type in Snowflake does not add its own scale that would show extra trailing zeros or truncate; either would make the displayed text differ from the library value even though the number is the same.

When an analyst says two numbers differ, first confirm they come from runs on the same side of the change date, then compare the stored value, then the displayed text.

## Interaction with replenishment orders

Replenishment order generation is the main purpose of ShelfSense. The rounding of `fill_rate` is meant for reporting and agreement with the dashboard. Order logic should not make hard threshold decisions on a rounded `fill_rate` if it can use the unrounded inputs instead. A store sitting right at a threshold could flip one way or the other because of rounding.

If any order rule currently compares `fill_rate` to a cutoff, review it. Either it is fine with the rounded value, or it should be fed from the counts. This has not been audited in full, and it is listed under Open questions.

## Testing the behavior

Tests should cover these cases, described in words so that the numbers are your own:

- A value with many digits comes out at 3 decimal places.
- A value that is already short is unchanged, with no padding that changes its meaning.
- A fill rate of none and a fill rate of everything both survive rounding unchanged.
- A midpoint case, once the tie convention is confirmed.
- A rollup computed from counts equals the rounded result of the unrounded ratio, and differs from the average of rounded children when the data is chosen to show it.
- Empty or zero-denominator input behaves as it did before and is not turned into a number by rounding.

Use Scala test code in the library's existing test layout, with a small local Spark session where a dataframe is needed.

## Null and zero-denominator handling

Rounding does not change how missing or undefined values are handled. When a denominator is zero or the inputs are missing, the library keeps whatever behavior it had before, which is to return a missing value rather than a fabricated one. Rounding a missing value must stay missing. Check that the rounding function used in Spark propagates nulls and does not coerce them.

## Compatibility notes for callers

Callers that compared `fill_rate` for exact equality against a stored value from before the change may now see mismatches. Fix those by rounding the old value the same way or by comparing with a tolerance that matches 3 decimal places.

Callers that format `fill_rate` as a percentage should be aware that a value at 3 decimal places gives a percentage with one decimal place at most. That is usually what analysts want. If a screen shows more digits than that, it is not using the library output directly.

## Rollout and verification

After the change is released, verify by taking a sample of stores from a recent run, reading `fill_rate` from the Delta Lake output and from Snowflake, and comparing both to the dashboard. They should be identical. Do the same for one rolled-up level so that the rollup rule is exercised.

If they disagree, work down the list: run date side of the boundary, column type in Snowflake, a job recomputing the formula outside the library, then the rounding mode.

## Open questions

- Which tie-breaking convention does the dashboard use at an exact midpoint, and does the library match it?
- Do any replenishment rules compare `fill_rate` to a cutoff, and should they use unrounded inputs?
- Should historical Delta Lake tables be backfilled, or is the mixed history acceptable?
- Does any other library metric need the same rounding to match the dashboard?
- Are there exports outside Snowflake that read library output and add their own formatting?

## Change log

This spec supersedes the earlier note about "shelf metrics must round". The current rule is that shelf-metrics-lib rounds `fill_rate` results to 3 decimal places. Update this section if the precision changes again, and record the reason, since the dashboard is the thing the value follows.
