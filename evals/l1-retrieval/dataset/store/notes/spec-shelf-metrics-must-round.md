---
id: 01KQGYJ52FDJHCY3N03ENVS10E
created: 2026-05-01T02:00-03:00
---

# shelf-metrics-lib spec: fill_rate rounding

shelf-metrics-lib is the shared Scala library that computes store-level shelf metrics for ShelfSense. The Spark jobs, the Airflow-scheduled batch runs and the Snowflake-facing exports all call it, so a metric has one definition and one numeric behaviour everywhere. This note pins down the rounding rule for fill_rate, why the rule lives in the library, and what callers and tests should expect. It is written in a hurry from what the team has settled, so it favours plain statements over polish.

The rule: shelf-metrics-lib must round `fill_rate` results to 4 decimal places. That applies to every value the library returns or writes for fill_rate, whatever the caller, whether it is a single store-item row, a store rollup, a category rollup or a chain-level figure. If someone asks how precisely fill_rate comes out of shelf-metrics-lib, the answer is 4 decimal places, and nothing in the library should return more digits than that or fewer.

## Why the rule exists

Merchandising analysts are the main readers of ShelfSense output. They compare fill_rate across stores, across weeks and across chains, and they paste figures into their own sheets and decks. When the same metric shows up with different precision in different places, they notice, and they stop trusting it. Before the rule was written down, the Spark output, the Snowflake tables and the replenishment order notes could each show a slightly different tail of digits for the same store and day. None of these differences was a real disagreement in the data. They came from where rounding happened, if it happened at all. The rule removes that class of complaint.

There is also a practical reason on the engineering side. The replenishment logic uses fill_rate as an input signal next to the stockout predictions. If one job feeds it raw floating point and another feeds it a rounded value, two runs over the same inputs can land on opposite sides of a threshold. That makes order generation hard to reproduce and hard to debug. A fixed precision, applied in one place, means two runs over the same inputs agree, and a threshold comparison is made against the same number an analyst can see on screen.

A third reason is storage and comparison. Delta Lake tables hold the metric and downstream consumers diff snapshots between runs. Unrounded doubles produce noise in those diffs, because tiny differences in summation order across Spark partitions change the last digits. Rounding to a fixed precision at the end of the computation hides that noise. It does not remove the need for care in how the sums are done.

## What the rule covers

The rule is about the value that leaves the library. It does not say how the library should compute intermediate quantities. Inside the library, numerator and denominator should be carried at full precision, and the division should happen once, at the end. Rounding to 4 decimal places is the last step. Rounding an intermediate quantity and then rounding again compounds the error, and it is the most likely way for someone to break this rule without meaning to.

Fill rate itself is a ratio: how much of what was wanted on the shelf was actually available. The library owns the definition of the numerator and denominator for each grain, and callers should not rebuild the ratio themselves. When a caller needs a rollup, it should ask the library for the rollup. It should not average already-rounded store-level values, because an average of rounded ratios is not the rounded ratio of the totals. The library computes the rollup from the underlying quantities and rounds once.

The rule applies the same way to every grain the library supports. A store-item-day value, a store-week value and a chain-month value are each rounded to 4 decimal places from their own unrounded ratio. A coarser grain is never derived from the rounded finer grain.

The rule does not cover other metrics. Other metrics in the library have their own precision decisions, and this note should not be read as setting them. If another metric needs a fixed precision, it gets its own statement, and it should be added here or in its own note rather than assumed from this one.

## Rounding behaviour

The library should use a single rounding helper for this, and every fill_rate code path should go through it. Do not scatter ad hoc rounding calls through the metric code. One helper means one place to change the behaviour and one place to test it.

The helper takes the unrounded ratio and returns a value at 4 decimal places. The tie-breaking mode should be a decimal-style half-up rule, not the binary floating point behaviour that surprises people on values that look like exact ties. For that reason the helper should do its rounding through an exact decimal representation rather than by multiplying, calling a floor or round function and dividing back on doubles. The multiply-round-divide trick on doubles gives the wrong answer on some values that sit close to a tie, and those are exactly the values an analyst will check by hand.

In Spark terms this means the rounding is expressed with the built-in decimal-aware rounding on columns where it can be, so that it runs inside the engine and is not done in a user-defined function. A user-defined function would block optimisation and would make the Scala and SQL paths drift apart. If a path really has to round on the driver side in plain Scala, it uses the same half-up rule on a decimal type, and a test ties the two paths together so they cannot silently diverge.

The output type matters as well. A value rounded to 4 decimal places but carried as a double can still print with extra digits after later arithmetic or a format conversion. Where the schema allows, the column should be a fixed-scale decimal type with scale matching the rule. Where the schema stays double for compatibility, the value is still rounded as described, and consumers are told that the stored double is the nearest representable value to the rounded decimal, not an exact decimal. Either way, the contract is the number of decimal places, and a reader who formats the value for display at that precision should always see the same digits the library produced.

## Edge cases

When the denominator is zero, there is no meaningful ratio. The library does not invent a number and does not return a rounded zero. It returns a null, so the absence is visible, and the rounding helper passes nulls through unchanged. Callers that need a default must apply it themselves and say so. A made-up zero looks like a total stockout, which would push replenishment orders for items that simply had no demand to measure.

When the ratio would exceed its natural upper bound because of data quirks such as late-arriving receipts or counting adjustments, the library should not silently clip it before rounding. Clipping hides data problems. The library rounds what it computed. If a value is out of range, a data quality check upstream or downstream should flag it. If the team decides the library should clip, that belongs in a separate, explicit statement in this spec and not as a side effect of rounding.

When the ratio is extremely close to a tie at the digit after the last kept one, the half-up rule on the exact decimal decides it. The tests should include a handful of such values chosen so that naive double arithmetic would get them wrong.

Very small positive ratios can round down to zero at this precision. That is accepted. The value then reads as zero even though some availability existed. Analysts looking for tiny non-zero rates should use the underlying quantities, which the library also exposes, and not the rounded ratio.

Negative values should not occur, because both parts of the ratio are counts or quantities that are not negative. If one appears, treat it as a data issue upstream, as with out-of-range values, and do not hide it in the rounding step.

## How callers should behave

Spark jobs that call shelf-metrics-lib take the result as is. They should not wrap it in another round call, cast it to a narrower type, or reformat it before writing to Delta Lake. A second round is harmless in value but misleading in intent, since it suggests the library is not trusted and invites a later change to a different precision at the call site. If a job needs a different presentation, it should do that at the very last display layer, and not in the data.

The Airflow tasks that orchestrate the batch runs should treat the precision as part of the contract between tasks. A task that reads fill_rate from Delta Lake and compares it to a threshold compares against the 4-decimal value as stored, and threshold constants in configuration should be written with no more precision than that, so a threshold can never sit between two representable values and behave oddly.

The Snowflake side receives the metric through the export step. The target column should have a fixed scale that matches the rule, so the warehouse never stores or shows more digits than the library produced and never rounds again with its own default. Any reconciliation between Delta Lake and Snowflake should compare for exact equality of the stored values, not within a tolerance, because both sides hold the same rounded numbers. A mismatch is a real defect in the export, and a tolerance would hide it.

Analysts see the figure in reports and in the replenishment order context. Those views should display the full precision the library gives and should not trim it to fewer places for tidiness. Trimming would bring back the original problem, where two screens show different digits for one fact.

## Testing

The library should carry unit tests that pin the behaviour so a later refactor cannot loosen it.

First, a plain-value test: a handful of ratios with known exact answers, checked to be returned at 4 decimal places. This covers the ordinary case and acts as the quick smoke test.

Second, tie-break tests: values chosen so the exact decimal sits on a tie just past the last kept digit, checked against the half-up expectation. These fail if someone swaps the decimal rounding for the multiply-round-divide trick on doubles.

Third, a rollup test: build a small set of store-level quantities, compute the chain-level rate through the library, and compare it with the rate computed by hand from the totals. Also assert that it differs from the average of the rounded store-level values on at least one crafted example, so the test documents why rollups must not be built from rounded parts.

Fourth, null and zero-denominator tests: confirm that a zero denominator yields null, that null passes through the rounding helper untouched, and that no path turns the null into a number.

Fifth, a schema test: confirm the output column type and scale are what the contract says, so a change to a more general numeric type is caught at build time and not discovered by an analyst.

Sixth, a Spark-versus-Scala parity test: run the same inputs through the column-expression path and the plain Scala path, if both exist, and assert equal results for every input. This is the guard against the two paths drifting.

These tests should run in the normal library build, not only in the heavier integration suite, so a violation fails fast.

## Changing the rule

The precision is a published behaviour of shelf-metrics-lib. Changing it changes numbers that analysts have in their own files, and it changes diffs against history in Delta Lake and Snowflake. Treat a change as a versioned, announced decision. It needs a note on why, a plan for existing stored values, and an agreed moment after which old and new values are not compared directly. Do not change it quietly in a bug fix.

If the team ever wants more or fewer places, the work includes the helper, the schema scale, the Snowflake column definitions, the threshold constants in orchestration configuration, the test expectations and this note. Missing any one of those brings back the inconsistency this rule was meant to end.

Until that happens, the statement stands: shelf-metrics-lib rounds `fill_rate` results to 4 decimal places, once, at the end, from the exact ratio, through one shared helper, and every consumer takes the value as given.

## Open points

Whether the library should offer an explicit clipped variant for callers who want a bounded rate is undecided. The current position is that it does not clip, and a clipped variant would be a separate, clearly named function with its own rounding behaviour described here.

The exact output type for the Spark path depends on how much downstream code still assumes a double. Moving to a fixed-scale decimal is preferred, but it should be done with a check of every consumer first, because some older reports may cast the column implicitly.

The documentation for analysts should say plainly that very small non-zero availability can display as zero at this precision, and point them to the underlying quantities. That text has not been written yet.

Finally, a short audit of existing call sites would be useful, to find any place that still rounds on its own or rebuilds the ratio from rounded parts. Anything found should be switched to the library helper and the library rollup functions, and the audit result added here.
