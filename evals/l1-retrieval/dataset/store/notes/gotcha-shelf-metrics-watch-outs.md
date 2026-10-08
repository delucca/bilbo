---
id: 01JRRJ0AADRSPWJ2VKNX5E2F68
created: 2025-04-13T18:48-03:00
---

# shelf-metrics-lib: things to watch when changing it

shelf-metrics-lib is the shared library that holds the metric definitions the rest of ShelfSense leans on. Stockout prediction, replenishment order generation and the analyst-facing reports all read numbers that come out of it. That is the whole problem with changing it: the code looks small and self-contained, but a change in one definition shows up in places that never import the library directly. This note is a list of general traps. It is not a spec and it records no decisions. Read it before you touch anything, and add to it when you get bitten by something new.

The short version: treat every metric as a contract with consumers you cannot see from inside the repository. Check the Spark side, the Delta Lake side, the Airflow side and the Snowflake side before you call a change safe.

## Who really depends on this library

The first mistake is assuming the callers are the ones in the same repository. They are not. Think of the data flow like this:

```
shelf-metrics-lib -> Spark job -> Delta Lake table -> Snowflake
```

The library is compiled in Scala and used inside Spark jobs. Those jobs write Delta Lake tables. Snowflake picks up from there, and analysts at the grocery chains see the result in their dashboards and in the generated replenishment orders. Airflow schedules the jobs in between. A change at the left end travels all the way to the right end, usually without any error on the way.

Things that follow from this:

- A metric that changes meaning does not fail anywhere. The job still runs, the table still has the same columns, the dashboard still renders. The numbers are just different. Nobody gets paged for that. An analyst notices a week later that orders look odd.
- Several jobs may use the same metric with different assumptions about its inputs. One job may pass in data that is already filtered to open stores, another may not. If you tighten an assumption inside the library, the second job can break or quietly produce nulls.
- Jobs run on their own schedule and may pick up the library at different times. For a while, two different versions of the same metric can be live in the same pipeline run window. Do not assume a clean cutover.
- Backfills and reruns are normal. A job that is rerun for an old period will use whatever library version it is deployed with now, not the one that produced the original numbers. Historical consistency is therefore your problem, not something the platform gives you.
- Downstream consumers outside the team sometimes copy a metric definition into their own SQL because it was easier than calling the library. Those copies will not follow your change. Look for them before you rename or redefine anything.

Before you start, write down for yourself every consumer you can find: jobs, tables, Snowflake views, dashboards, anything that builds an order from the output. If you cannot name them, say so in the pull request instead of pretending the list is complete.

## Metric semantics: the quiet breakage

Most of the real danger is in what a metric means, not in how it is coded. A few areas that have caused trouble or are obviously going to.

### Denominators, windows and what counts as a day

Rates and ratios are the easiest place to introduce a silent shift. When you change what goes in the denominator, the metric moves for every store, and the move is not uniform. Stores with sparse sales history shift more than busy ones. Check these every time:

- What is the window, and is it anchored to the calendar, to the store's own trading days, or to the number of days since a product was listed? Changing the anchor changes the value even if the window length stays put.
- How are days with no sales treated? A zero and a missing row are different things. A store that was closed, a product that was delisted, a shelf that was empty, and a product that simply did not sell all look similar in a sales table, and they mean four different things to a stockout model.
- How are partial days handled? Stores that open late, close early or have a data feed that cuts out part way through the day produce short days. If a rolling average treats a short day like a full day, you get a downward bias that looks like weak demand.
- Where does the day boundary sit? Time zones matter in a grocery chain that spans regions. If the library computes day boundaries in one zone and the source data was bucketed in another, a store near the boundary will have sales fall into the wrong day. Be careful when touching anything that truncates a timestamp to a date.
- Holidays, promotions and store closures change demand patterns. If a metric tries to be robust to those, a change to the robustness logic can flip which days are excluded.

### Stockout definitions

A stockout is not one thing in this domain. It can mean zero on-hand according to the inventory system, zero sales for a stretch of time on an item that normally sells, a shelf that is physically empty while stock sits in the back room, or a phantom inventory case where the system says stock exists and the shelf says otherwise. The library may encode one or more of these. Be very explicit about which one your change touches. If you add a new notion of stockout, give it a new name. Do not reuse the old name with a different meaning, because consumers will pick it up without reading your description.

Also be careful about the direction of errors. Over-flagging stockouts causes unnecessary orders and overstock, which in grocery means waste for perishables. Under-flagging causes empty shelves and lost sales. These are not symmetric costs, and they differ by category. A change that looks neutral on average can still be bad for fresh goods while harmless for shelf-stable goods. Look at categories separately when you evaluate a change, not just the overall figure.

### Units

Quantities in grocery data come in many units: each, case, pack, weight, volume. The library may convert between them, or it may assume the caller already did. Never assume which. When you add a metric, state the unit in its name or its documentation. When you change a conversion, check every place where a quantity is summed across items, because summing across different units gives plausible-looking nonsense.

Weighted items, sold by weight at the till, are a classic source of trouble. A count of transactions and a count of units are not the same for them, and any metric that treats the two as interchangeable will be off for produce, deli and meat.

### Rounding and precision

Be deliberate about rounding. Rounding early inside a chain of calculations compounds. Rounding at the end of the chain can still change a replenishment order, because order quantities are often rounded up to pack sizes, and a tiny difference near a boundary tips the order by a whole pack. If you change numeric types, such as moving from floating point to decimal or the other way, expect small differences everywhere and decide ahead of time whether they are acceptable. Do not call small differences noise without checking whether they land near ordering thresholds.

### Nulls

Spark's null handling is the single most common source of surprise. Aggregates ignore nulls, comparisons with null give null rather than false, and a filter on a null condition drops the row. In a metric library this means:

- A new column with some nulls can change the result of an existing aggregate without any code change on the aggregate.
- A join that used to match every row may start to drop rows when a key becomes nullable.
- A default value for a missing input is a decision about meaning, not a convenience. Filling with zero, filling with the previous value and leaving null are three different statements about the world.

If your change introduces, removes or relocates a null, say so in the description and check the consumers.

## Spark and Scala specifics

The library is Scala running on Spark, so the usual traps for that combination apply, and some of them are easier to hit in a shared library than in a single job.

### Binary and source compatibility

Scala is stricter than most people remember about compatibility. Changing a method signature, adding a parameter with a default, changing a case class, moving something to another package or changing an implicit can break callers at compile time or, worse, at run time on a cluster where jobs were compiled against an older shape of the library. Things to watch:

- Case classes are used for convenience and then become part of the public surface. Adding a field to one changes its constructor, its copy method and its pattern matches. Callers that deconstruct it will break.
- Default arguments look backward compatible and often are not, at the binary level. If jobs are not all rebuilt at the same time, you can get missing method errors at run time on only some of them.
- Implicits are resolved at compile time in the caller. A new implicit in the library can change which one a caller picks up, or make a previously unambiguous call ambiguous.
- Sealed traits and enumerations: adding a case can turn a once-exhaustive match in a caller into a match that fails at run time.
- Changing the Scala version or a Spark version used to build the library is its own project. The compiled artifact is tied to both. Do not slip a build upgrade into a metric change.

If you must make a breaking change, prefer adding the new thing next to the old one, marking the old one deprecated, and moving consumers over before removing it. Removal is a separate change.

### Serialization and closures

Code in the library often runs inside Spark tasks. Anything captured by a closure has to be serializable, and anything big that gets captured gets shipped to every task. Common ways to break this:

- Referencing a field of an enclosing object from inside a lambda, which drags the whole object into the closure.
- Holding a non-serializable helper, such as a formatter or a client, as a field and using it in a transformation.
- Putting a large lookup structure into a closure instead of broadcasting it.

These problems often show up only on a real cluster, not in a small local test. A change that passes unit tests can still fail when it runs distributed. Where you can, exercise the changed code path in a cluster-like setup before merging.

### User-defined functions

If a metric is implemented as a user-defined function, remember that Spark treats it as a black box. The optimizer cannot push filters through it, cannot prune columns around it, and cannot see that it is deterministic unless you say so. Replacing a built-in expression with a UDF, or the other way around, can change performance by a lot and can change null behavior. A UDF that is marked deterministic but is not will produce different results across retries. Prefer built-in column expressions when they can express the metric. If you convert one to the other, compare the physical plans, not only the output.

### Partitioning, shuffles and skew

Grocery data is skewed. A handful of high-volume items and stores carry a lot of rows, and a chain's biggest stores dwarf the small ones. When you add a join, a group by or a window to a library function, you are adding a shuffle to every job that calls it. Think about:

- Which key you group on, and whether one value of that key can hold a large share of the rows.
- Window functions that partition by a coarse key and sort within it. These can blow up on a single partition.
- Wide joins between a large fact table and a dimension that has grown. A join that was safe as a broadcast may no longer fit.
- Explode operations that multiply rows, for example when generating a row per day per item per store.

The library cannot know the size of the caller's data. If a function is expensive, say so in its documentation, and avoid hiding a shuffle inside something that looks like a cheap column expression.

### Determinism and ordering

Some metrics depend on row order, such as "last known value" or "previous day". Spark does not guarantee order unless you sort, and a sort across partitions is itself a shuffle. If you use first, last, collect_list or anything similar, check that the ordering is explicitly specified. A result that is stable on a small test and flips on a large run is typically this problem. Also watch for functions that use randomness or the current time. A metric that reads the clock inside the library makes reruns non-reproducible. Pass the as-of time in from the caller.

### Schema handling

The library probably expects certain input columns with certain types. Be careful with:

- Relying on column names that callers might alias differently.
- Type widening and narrowing. An integer sum that overflows, or a decimal whose precision and scale shift after arithmetic, can silently truncate or return null depending on settings.
- Implicit casts between string and date or timestamp types, which can be sensitive to session settings.
- Struct and array columns, where field order matters in some operations.

Prefer failing early with a clear check on the incoming schema over letting a wrong type travel three steps and fail somewhere confusing.

## Delta Lake and table contracts

The output tables are Delta Lake tables, and other things read them. Treat their schema and their semantics as a published interface.

- Adding a column is usually safe for readers, but not always. Consumers that select everything and then positionally map columns can break. Snowflake loads that depend on column order or a fixed set of columns can reject or misplace data.
- Renaming or dropping a column is a breaking change, even though Delta Lake can do it. Anything downstream that reads by name will either fail or, if it tolerates missing columns, quietly return nulls.
- Changing a column's type is a breaking change, including widening. Check how Snowflake maps the type on the way over.
- Changing the partitioning of a table changes how every reader performs and how every writer behaves. It is not a library-only concern, and it needs coordination with whoever owns the table.
- Schema evolution settings on writers matter. If a job is set to merge schema automatically, a library change can alter a production table without anyone deciding to. If it is not, the job will fail on the first run after the change. Know which one you have before you merge.
- Merge and upsert logic depends on keys. If a metric's grain changes, for example from per store per day to per store per item per day, the keys change, and an upsert on the old keys will either duplicate rows or overwrite the wrong ones.

### Grain and uniqueness

Write down the grain of every table your change touches: what one row represents. Then check that the change keeps it. Metrics that join on the wrong grain are a classic source of duplicated rows, which in turn inflate sums. A cheap sanity check is to count rows by the intended key and confirm there are no duplicates, before and after the change.

### Time travel, retention and history

Delta Lake keeps history, which tempts people to think a bad change can always be undone. That works for the table, not for what consumers did with it. Orders generated from bad numbers have already gone out. Snowflake copies have already been refreshed. Retention windows also run out, and cleanup of old files removes the ability to go back. Do not rely on time travel as the plan for a risky change.

If a metric is redefined, decide what happens to history. Options include leaving old rows as they were, recomputing them, or marking a break in the series. Each choice affects trend charts and model training. Whatever you pick, make it explicit, and tell the people who read trend lines. A model trained on a mix of old and new definitions learns the definition change as if it were real demand.

### Idempotence and reruns

Jobs get rerun after failures. The code you change in the library must produce the same output when run twice over the same input, and the write step must not duplicate data. Check that nothing in the changed path depends on the current time, on a mutable external lookup, or on the state of the target table in a way that makes the second run differ from the first.

## Airflow, Snowflake and what happens around the library

### Scheduling and ordering

Airflow decides when jobs run and in what order. A change in the library can change how long a job takes, and therefore whether it still finishes before the jobs that depend on it. A metric that gets slower because of a new shuffle can push a downstream task past its window, and the symptom shows up as a late or missing order file, far from the cause.

- If you make a function noticeably heavier, flag it to whoever owns the schedule.
- If a change needs two jobs to be updated together, think about what happens if one runs on the new version and the other on the old. Airflow does not make that atomic for you.
- Retries and backfills run the code that is deployed at that moment. A backfill started during a rollout may mix versions across dates.
- Sensors and dependencies sometimes key off the existence of an output partition or a table update. If a change alters when or how that output appears, the sensors may wait forever or trigger too early.

### Snowflake

Snowflake is where much of the analyst-facing work happens. Think about:

- Type mapping from Delta Lake to Snowflake. Decimals, timestamps with and without time zone, and nested types all have rules that can differ from what Spark does.
- Case sensitivity of column names. A rename that only changes case can be harmless in one system and break in the other.
- Views and queries written by analysts. They may select specific columns or depend on a particular meaning. Dropping or redefining one breaks something you have never seen.
- Cost. A change that makes tables wider or much larger has a cost downstream. A metric stored at a finer grain than before is a bigger load and bigger scans.
- Refresh timing. If Snowflake data is refreshed on its own schedule, a library change may be visible in one place and not yet in the other. People comparing the two will think one is wrong.

### Order generation

Replenishment orders are the point where a wrong number turns into something physical: trucks, labor, product that may spoil. This is the consumer to think about hardest. Before merging a metric change, ask what the order logic does with the number. Is it compared to a threshold? Is it multiplied by a lead time? Is it rounded to a pack size? Does it cap or floor the result? A small shift can cross a threshold for many items at once and produce a visible jump in order volume. If you can, compare the orders generated from old and new definitions on the same input and look at the differences by category and by store type, before the change goes to production.

## Testing and validation

Unit tests in the library are necessary and not sufficient. They check that the code does what its author thought. They do not check that the author thought the right thing, and they do not see the consumers.

### What to test

- Edge cases of the domain: a store that is closed for part of the window, an item that was just introduced, an item that was just delisted, a day with no data at all, a day with returns larger than sales, negative inventory adjustments, and items with a very long history of no sales.
- Null and missing input for every argument that can plausibly be null.
- Duplicate input rows, since upstream feeds do send duplicates and rerun feeds send them twice.
- Skewed input, where one key holds almost everything.
- Both ends of the unit range, such as items sold by weight and items sold by pack.
- Idempotence: running the function twice on the same input gives the same result.

### Compare before and after on real-shaped data

The most useful check for a metric change is a side-by-side run of the old and new code on a representative sample of data, followed by looking at where they differ. Do not stop at the overall mean. Look at:

- The distribution, not only the average. A change can leave the mean alone and fatten a tail.
- Breakdown by category, store format and region.
- The rows with the largest differences. Open a few and check by hand that the new answer is the right one.
- The count of rows that changed from null to non-null or the reverse.
- Whether any item or store crossed an ordering threshold because of the change.

Sample data that is too clean hides problems. Make sure the sample includes messy stores, new items and promotion periods.

### Do not let the tests drift

A common failure is updating the expected values in a test to match the new output without asking whether the new output is right. If a test fails after your change, read the failure as a question. Sometimes the test is outdated; often it caught a real shift. If you change an expected value, say why in the review.

Also be wary of tests that pass because the test data is too small to trigger the shuffle, the skew or the serialization path. A local run in a small session does not behave like a cluster.

### Review

Ask someone who understands the merchandising side to read metric definition changes, not only someone who reads Scala. Code review will catch bugs. It will not catch a definition that is coded correctly and means the wrong thing. If there is no such person available, write a plain-language description of what the metric means before and after, and put it in the pull request, so a later reader can tell whether the meaning shifted on purpose.

## Rollout, versioning and communication

### Make changes small and separable

Keep metric changes apart from refactors, build changes and dependency upgrades. When something goes wrong after a combined change, you cannot tell which part did it. A pure refactor should produce byte-identical output on the same input; if it does not, it was not a pure refactor. Check that explicitly.

### Prefer additive changes

When a metric needs to change meaning, add a new one beside the old and let consumers move across. Name it so that nobody confuses the two. Keep the old one for a period, mark it deprecated in a way people will see, and remove it only after you have checked who still uses it. Changing a definition in place is the fastest route to the quietest outage.

### Versioning

Know how the library is versioned and how jobs pick up a version. If jobs pin a version, a release does not take effect until they are bumped, and you must follow up on who has not moved. If they pick up the latest automatically, a release takes effect on the next run everywhere, and your change is effectively a production deploy. Either way, version bumps should reflect whether a change is breaking, and release notes should describe changes in terms of meaning, not just code.

### Feature flags and gradual rollout

Where possible, put a risky change behind a switch that the caller controls, so one job can adopt it first and the rest later. Compare the outputs of the adopting job against the others for a while. Remove the switch once everything has moved; leftover switches turn into permanent branches that nobody dares to delete.

### Rollback

Think through rollback before you ship, not after. Reverting the library is simple. Cleaning up what the new version wrote to tables, what Snowflake ingested and what orders were created is not. Decide what you would do about the bad window of data, who would need to know, and whether orders would need to be held or corrected. If you cannot answer that, make the change smaller or put it behind a switch.

### Tell people

Analysts and whoever owns ordering need to hear about any change in metric meaning before it lands, in words they can follow: what changes, which numbers will move, in which direction, and roughly for whom. A metric that shifts without warning gets reported as a bug in the forecast, and time gets spent hunting for a model problem that is really a definition change. Also note the date of the change in whatever changelog consumers read, so trend breaks can be explained later.

## Smaller habits that save time

- Read the existing function and its callers before editing. Search for usages across all the repositories you can reach, not only this one, and include SQL and notebooks.
- Do not trust names. A function with a name that suggests one thing may do another. Read the body.
- Do not trust comments either, especially older ones. Check them against the code and fix them if they are wrong.
- When you find odd behavior that looks like a bug, check before fixing it. Someone may rely on it, and fixing it is a metric change like any other.
- Keep logging in the library quiet. Code that runs inside tasks and logs per row will flood executors and slow jobs down.
- Do not add a dependency casually. Every library you add to shelf-metrics-lib is added to every job that uses it, and version conflicts with the Spark runtime are painful to chase.
- Avoid hidden global state, such as mutable singletons or settings read from the environment. They behave differently between local runs, tests and clusters.
- Keep configuration explicit. If a metric has a tunable parameter, pass it in, and name where the default came from. Hidden defaults are a decision in disguise.
- When something looks wrong in the output, check the input first. Upstream feeds have late data, duplicate data and gaps, and a lot of what looks like a library bug is a feed problem. The reverse also happens, so check both.
- Write down what surprised you. If you hit a trap that is not on this list, add it here in general terms, so the next person does not repeat it.

## Checklist before merging

Quick pass, in order:

1. Did the meaning of any metric change? If yes, is it a new name rather than an in-place edit, and has someone from the merchandising side seen the description?
2. Did the signature, a case class, an implicit or a sealed type change? If yes, is it compatible for jobs not rebuilt yet?
3. Did the output schema or grain of any Delta Lake table change? If yes, have the Snowflake side and the table owners been told?
4. Does the change add a shuffle, a UDF or a wide join? If yes, have you looked at the plan and thought about skew?
5. Are nulls, duplicates, empty windows and short days handled, and tested?
6. Is the function deterministic and safe to rerun, with the as-of time coming from the caller?
7. Did you compare old and new outputs on realistic data, by category, and look at the largest differences?
8. Did you check what the order logic does with the changed numbers?
9. Is there a plan to roll back, including cleanup of data already written and orders already sent?
10. Are the people who read the numbers going to hear about it before they see it?

If any answer is no or unknown, say so in the pull request. An honest unknown is better than a confident guess, because someone else may know the answer.
