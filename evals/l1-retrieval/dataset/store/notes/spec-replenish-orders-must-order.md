---
id: 01KQX34Q4WRPWXMCWG22RN9NC9
created: 2026-05-05T19:11-03:00
---

# replenish-orders-job spec

This is the working spec for replenish-orders-job, the Spark job in ShelfSense that turns store-level stockout predictions into replenishment orders. It is written in a hurry from what the team has settled, so it is blunt. Where something is not settled it says so. If you change behavior in the job, change this note too, because analysts read the output and ask why a quantity looks the way it does, and this note is where the answer should be.

The two rules that matter most, stated up front so nobody has to hunt for them: replenish-orders-job must cap each order line at 300 units, and it must round quantities up to a multiple of `case_pack_qty`. Everything below is context for those two rules, how they interact, and what else the job does around them.

## What the job is for

ShelfSense predicts, per store and per item, the chance that the shelf runs empty before the next delivery. Those predictions come out of the forecasting side of the project and land in Delta Lake tables. replenish-orders-job is the step after that. It reads the predictions, reads current inventory and pending inbound supply, decides how much of each item each store should be sent, and writes order lines that the downstream ordering system picks up. Merchandising analysts at the grocery chains look at those lines in Snowflake, adjust or approve them, and the chain's own ordering process takes it from there.

The job is written in Scala on Apache Spark. It is scheduled by Airflow, runs after the prediction tables for the day are complete, and writes its results to Delta Lake. A copy of the final order lines is made available to Snowflake for the analysts. The job does not talk to suppliers or warehouses directly. It produces lines, and that is all it does. Keep it that way: every time someone suggests adding a push to a warehouse system from inside this job, the answer has been no, because retries and partial failures get ugly fast.

The job is not the forecaster. It does not retrain anything and it does not try to improve the stockout probabilities. If the quantities look wrong because the probabilities look wrong, the fix is upstream. It is also not the place where store-level business policy lives in general. The only policy it owns is the quantity logic described here: how much to order, capped, and rounded to what the supplier can actually ship.

### Who reads the output

Analysts care about three things in the output. First, whether the line is sensible for the store, meaning the quantity is not wildly more than the store sells. Second, whether the quantity is shippable, meaning it is a whole number of cases. Third, whether they can tell why a line is what it is. The third one is why the job keeps a reason field on each line, described later. Analysts do not read Scala, so the reason field and this note are the explanation they get.

## Quantity rules

This section is the core of the spec. The order of operations matters, because cap and rounding interact, and the interaction has bitten people.

### Raw need

The job starts from a raw need per store and item. Raw need is the quantity the store is expected to want over the coverage window, minus what is on hand, minus what is already inbound, floored at zero. The coverage window is the time until the next delivery opportunity plus a safety allowance that scales with how likely a stockout is. Items with a high stockout probability get a bigger allowance; items with a low probability get almost none. If raw need comes out zero or negative, no line is written for that store and item at all. A zero-quantity line is noise and the analysts complained about it early on, so the job drops them rather than writing them.

Fractional raw need is normal, because forecasts are rates. The job carries the fraction through until rounding. Do not round early. Rounding raw need to a whole unit before the case-pack step throws away information and has produced off-by-a-case errors.

### Rounding to case packs

Quantities are rounded up to a multiple of `case_pack_qty`. The column comes from the item master data joined onto each candidate line, and it says how many units come in a case from the supplier. Suppliers ship whole cases, so a line that is not a multiple would either be rejected or silently rounded by someone else, and we would rather do it ourselves and be able to explain it.

Rounding is always up, never to nearest and never down. The reasoning is that the purpose of the order is to prevent a stockout, and rounding down would quietly under-serve exactly the items the model flagged as at risk. The cost of rounding up is a little extra stock, which the analysts accepted. If raw need is positive but small, the line becomes one full case, not zero. This is deliberate. Do not "fix" it.

If `case_pack_qty` is missing, null, or not positive for an item, the job does not guess. See the section on bad data below. The short version is that the line is held out of the main output and flagged, not defaulted to a pack size of one.

### The cap

Each order line is capped at 300 units. The cap is per line, meaning per store and item and delivery, not per store and not per item across the chain. A store can have many lines, and the same item can appear on lines for many stores, each with its own cap. The cap exists because early versions occasionally produced absurd lines for items with a bad forecast spike, and one such line can swamp a store's backroom or blow through a supplier allocation. The cap is the last line of defense against that, not a substitute for good forecasts.

The cap is a hard limit on the final quantity. That means the final quantity written must never exceed 300 units, after rounding. This is the part that trips people up, so here is how the two rules combine.

### How cap and rounding combine

The cap is applied to the final, rounded quantity, and the final quantity must be both a multiple of `case_pack_qty` and no more than 300 units. If you round up first and then clamp to 300 units, you can land on a number that is not a multiple of the case pack. For example, if the pack size does not divide the cap evenly, clamping the rounded value straight to the cap breaks the multiple rule. Clamping first and then rounding up has the opposite problem, because rounding can push the line back over the cap.

The agreed behavior is: round up to a multiple of `case_pack_qty`, and if that exceeds the cap, step down to the largest multiple of `case_pack_qty` that does not exceed 300 units. So the cap wins over the round-up direction, and the multiple rule wins over the exact cap value. The result is always a whole number of cases and never above the cap. When the step down happens, the reason field on the line says the cap was applied, so an analyst can see that the store wanted more than was written.

There is one awkward case: an item whose case pack is itself larger than the cap. Then there is no multiple of the pack that fits under 300 units except zero. The decision so far is that the job does not write a line for it, and it does not write an over-cap line either. It reports the item in the held-out set with a reason saying the pack is bigger than the cap, so a human can decide whether to raise the cap for that item or ignore it. Nobody has asked for a per-item cap override yet. If someone does, it should be a column in the item master data or a small config table, not a constant buried in the Scala code.

### Why not a lower or higher cap

People ask. A higher cap lets bad forecasts through. A lower cap truncates legitimate orders for fast sellers in big stores and the analysts then manually top up, which defeats the point of the job. The current value was picked as a compromise after looking at the distribution of legitimate lines and the outliers; legitimate lines were overwhelmingly well below it and the outliers were far above. If the distribution shifts, revisit it, but change it in config and update this note, not by editing the constant in one place.

### Order of operations, in words

To keep it straight, this is the sequence the job follows for each candidate line. Compute raw need as a fractional quantity. Drop the line if raw need is not positive. Look up the case pack and hold the line out if it is unusable. Round the need up to a whole number of cases and convert back to units. If the result is over the cap, step down to the largest whole number of cases that fits under 300 units, and mark the line as capped. If stepping down leaves nothing, hold the line out with the pack-bigger-than-cap reason. Write the line with its reason.

## Inputs, outputs and flow

### Inputs

The job reads four kinds of data. Stockout predictions per store and item from the forecasting tables. Current inventory per store and item, from the inventory snapshot that the chains send us. Pending inbound supply, meaning orders already placed that have not arrived. And item master data, which carries the case pack and a few other attributes such as whether an item is active at a store. All of these live in Delta Lake, and the job reads them as of the run's logical date so that reruns are reproducible.

The inventory snapshot is the weakest input. Chains send it at different times, and some send partial files. The job does not try to repair a missing snapshot. If a store has no snapshot for the run date, it is skipped and listed in the held-out output with a reason. Ordering for a store on stale inventory is worse than not ordering, because it double-orders items the store already received.

Inbound supply matters because without it the job would reorder things that are already on a truck. The subtraction is done per store and item. If inbound data is late or missing for a chain, the job treats that chain as missing inputs rather than assuming nothing is inbound. This was a decision, not an accident: assuming nothing inbound produced a wave of duplicate orders in an early trial.

### Outputs

The main output is a Delta table of order lines. Each line carries the store, the item, the final quantity in units, the case count, the run date, and a reason field. The reason field is a short code plus a readable string that says what drove the quantity: normal, capped, rounded up from a tiny need, and so on. Analysts filter on it. Keep the set of reason codes small and stable, because dashboards in Snowflake depend on them.

The second output is the held-out table. It holds everything the job decided not to write as an order line: missing or unusable pack sizes, pack bigger than the cap, stores with missing inventory snapshots, and items inactive at the store. Each row has a reason. This table is how the job fails loudly without failing the run. A run that held out a few lines is a normal run. A run that held out a large share of a chain's lines is something the on-call person should look at, and there is a check in the Airflow DAG that warns when the share is high.

The output is written idempotently per run date: a rerun for the same date replaces that date's lines rather than appending. The write is a Delta merge or an overwrite of the date partition, whichever the current code uses, and the important property is that running the job twice does not double the orders. Downstream systems read the latest version for the date.

### Scheduling

Airflow runs the job once a day after the prediction tables and the inventory snapshots are in place. Sensors wait on those tables. After the job succeeds, a downstream task refreshes the Snowflake copy that analysts use. If the job fails, the refresh does not run, so analysts see yesterday's lines rather than a half-written set. The DAG is deliberately simple. Resist adding branching for individual chains inside it; chain-specific differences belong in data, not in DAG structure.

Backfills are allowed and are just reruns for past dates. Because inputs are read as of the logical date, a backfill reproduces what the job would have written then, subject to the rule in the code at that time. If the rules change, old output is not rewritten unless someone explicitly reruns it.

## Bad data and edge cases

This section collects the cases that have actually come up, so the next person does not rediscover them.

### Case pack problems

A missing, null, zero or negative `case_pack_qty` is treated as unusable. The line is not defaulted to a pack of one, because that would write odd unit quantities to suppliers who only ship cases, and the order would be bounced or silently adjusted. The line goes to the held-out table with a reason naming the pack problem. The fix is in the item master data, which is owned by a different team, so the held-out table is also what we send them when they ask what is broken.

A pack size that changes over time is a real thing: suppliers change pack configurations. The job uses the pack size as of the run date. It does not try to be clever about transitions. If a pack changes mid-flight and inbound orders were placed under the old pack, the inbound units are still counted in units, so the subtraction stays correct.

A non-integer pack size should not exist, but the data has had a few. The job treats a non-integer pack as unusable too, rather than rounding the pack itself. Rounding a pack size would change the meaning of the whole line.

### Cap edge cases

Besides the pack-bigger-than-cap case described above, there is the case where the capped line is much smaller than raw need. That is allowed, and the job writes the capped line with the capped reason. It does not spill the remainder onto a second line for the same store and item. Splitting a line to dodge the cap would defeat its purpose. If a store genuinely needs more than a single line allows, the analysts handle it by hand, and the capped reason is their signal that this happened.

Because the cap is per line, someone occasionally asks whether the cap should apply across stores for a shared supplier allocation. It should not, in this job. Allocation across stores is a different problem with different inputs, and nobody has specced it.

### Rounding edge cases

A raw need that is exactly a whole number of cases stays at that number of cases. Rounding up must not add a case when the need is already a multiple. Be careful with floating point here: a need that is mathematically an exact multiple can come out as a hair above it in floating point and then round up to an extra case. The job guards against this with a small tolerance before the ceiling operation. If you touch the rounding code, keep the tolerance and keep it small. A tolerance that is too large would swallow real fractions and round down when the rule says up.

Very small positive needs become one case, as noted. This produces a visible amount of "extra" stock for slow movers with large packs. Analysts have asked about it. The answer is that it is by design, and that if they want to suppress the tiny lines they should do it in review using the reason field, not by asking the job to round down.

### Inactive and delisted items

Items that are not active at a store are dropped before any quantity logic and listed in the held-out table. This also covers items delisted since the prediction was made. The prediction tables sometimes lag the assortment, and ordering a delisted item is a waste and an annoyance for the chain.

### Duplicates

If the same store and item shows up twice in the candidate set, which has happened when upstream tables were rerun and not deduplicated, the job keeps one candidate before quantity logic, using the latest prediction. It does not add them. Summing duplicates doubled orders once and the cap hid it only partly, since two capped lines can still be a lot.

## Related notes and open items

There is a related note about how rounding was handled in the reporting metrics: [[shelf-metrics-must-round-revised]]. It covers the metrics side, where the question was how numbers shown to analysts get rounded. Do not confuse that with the rule here. The rounding in replenish-orders-job is always upward to a case multiple and is about shippable quantities. The metrics rounding is about display. If a metric and an order line disagree by a rounding step, check which of the two rounding rules produced each number before assuming a bug.

### Things that are decided

The cap is per line and applies to the final quantity. Rounding is always up, to a multiple of `case_pack_qty`. When the two conflict, the result is the largest multiple of the pack that fits under the cap. Unusable packs and over-cap packs are held out, not defaulted or forced through. Zero-need lines are not written. Reruns replace, they do not append. Chains with missing inputs are skipped, not guessed.

### Things that are not decided

Per-item cap overrides: no design yet, and no clear owner. If it comes up, store the override with the item master data rather than in code.

Whether tiny lines for slow movers should be suppressed automatically instead of rounded to a full case: the analysts are split. For now the behavior stays as is.

Whether the held-out table should feed an alert to the data owners of the item master data directly, instead of the on-call person forwarding it by hand. It would save time, but it adds a dependency on another team's tooling.

Whether the safety allowance should be tuned per chain. Right now it is a single formula. Some chains have much more reliable delivery than others, and a single allowance over-orders for them. This is a forecasting-and-policy question more than a job question, and it should be settled before anything is changed in the code.

### Working notes for whoever edits the job

Keep the quantity logic in one place, a small pure function that takes raw need, pack size and cap and returns the final quantity and a reason. That makes it easy to read and easy to argue about. Do not scatter the cap and rounding across several transformations in the Spark pipeline, because the interaction between them is the whole point and it is hard to see when it is spread out.

Keep the cap value and the tolerance in configuration, with the cap documented as being in units. Do not hardcode either deep inside a transformation. When the cap changes, update this note and tell the analysts, because it changes what they see in review.

When adding a reason code, add it to the shared list that the Snowflake dashboards read from, and add a line for it here. A reason code that exists only in the job is invisible to the people who most need it.

If a run looks wrong, look at the held-out table first. Most surprises, such as a store with no lines or an item that is missing, turn out to be a held-out row with a clear reason, not a bug in the quantity logic. The quantity logic itself is small and has been stable; the data around it is what moves.
