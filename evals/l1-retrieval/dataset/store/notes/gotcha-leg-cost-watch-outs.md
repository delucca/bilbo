---
id: 01KWNNBAHCRJ6B1R72BAFJG25X
created: 2026-07-04T01:14-03:00
---

# leg-cost-matrix: things to watch out for when changing it

This is a general list of traps around leg-cost-matrix. It is not a spec and it records no decisions. Read it before touching the component, and add to it when something new bites. Most of the items come from the same cause: leg-cost-matrix looks like a plain table of costs, but a lot of other parts of FreightWeave treat it as a contract, and the contract is mostly unwritten.

The short version: the matrix is shared state between the planner, the rebalancer, the cache and the event feed. A change that is correct in isolation can still shift route choices, invalidate cached data, or make a rebalance disagree with the plan it is supposed to fix. Check all of those before calling a change done.

## What leg-cost-matrix actually is

It holds the cost of moving a load across one leg, for the combinations of origin, destination, mode and carrier class that the planner may consider. The solver reads it as input. Nothing in the solver knows what the entries mean in business terms; it just minimizes a sum. So whatever meaning the entries carry has to be consistent in the matrix itself.

A few things people assume that are worth stating:

- The entries are a blend. Distance, time, handling, waiting, tolls, and penalties can all be folded into one number. If you change how one ingredient is computed, you change the balance between all the others, even if you did not touch them.
- The matrix is not symmetric in general. A leg in one direction is not the same as the reverse. Rail in particular has direction-dependent behavior, and truck legs differ by empty running and backhaul availability. Do not add a shortcut that mirrors entries.
- Missing entries mean something. A missing pair is different from a very expensive pair. Some code paths treat absence as not allowed, others treat it as not yet computed. Know which one you are in before you fill gaps.
- It is rebuilt and partially updated at different times. The full build and the incremental update paths do not always go through the same code, so a fix in one can leave the other wrong.

If you are new to the component, read the build path and the update path side by side before editing either. They should agree, and sometimes they do not.

## Units and scaling

The solver works on integers. Costs get scaled and rounded on the way in. That scaling is the most common source of quiet bugs here.

- Rounding happens somewhere between the raw cost and what the solver sees. If you change where the rounding happens, ties between routes break differently. Dispatchers notice when a familiar route flips for no obvious reason.
- Mixed units are easy to introduce. Time in one unit, distance in another, money in a third. When a new ingredient is added, convert it into the same cost unit as everything else, and check that its scale is sensible next to the others. An ingredient that is far too large will dominate everything and the solver will still happily produce an answer.
- Large cost values can overflow or lose precision on the solver side. Penalties meant to say avoid this leg are a particular risk. Using a huge sentinel makes sums across a route blow up. Prefer excluding the leg over a giant penalty, and keep any sentinel well inside what the solver can add up over a long route.
- Negative costs are a trap. Credits and backhaul incentives are tempting to model as negatives, but many solver setups assume nonnegative arc costs, and negative cycles break shortest-path style reasoning. If you need an incentive, model it some other way and test on a multi-leg route.
- Do not switch between floats and integers midway through the pipeline. A float that is cast late gives different results than one cast early, and the difference only shows up on long routes where the errors add up.

When you change anything numeric, compare the old and new matrix on a realistic sample and look at the distribution of changes, not just whether tests pass. A uniform shift is usually harmless. A shift that hits only some modes or some regions is a behavior change and should be treated as one.

## Mode differences: truck and rail

Truck and rail legs share the matrix but they are not the same kind of thing.

- Rail has schedule structure. A rail leg cost is only meaningful together with a departure window, cutoffs, and transfer time at terminals. If the matrix stores a flat rail cost, then the waiting component is baked in from some assumption. Changing that assumption changes how attractive rail is across the whole network.
- Truck costs depend on driver hours rules, empty repositioning, and congestion by time of day. These are often approximated. Do not tighten an approximation in one place and leave the matching approximation elsewhere, or the planner will prefer whichever side is more optimistic.
- Intermodal transfer cost belongs at the joint between legs, not inside either leg. If a change moves transfer cost into a leg, routes that avoid the transfer get cheaper than they should, or the reverse. Check where transfer handling lives before editing.
- Adding a new mode or a new carrier class changes the shape of the matrix. Code that iterates over modes, or that builds keys from mode, may silently skip the new one or collide with an existing key. Search for every place that enumerates modes.

If a change is described as only affecting rail, run a truck-only sample as well and confirm nothing moved. Shared helper functions make this easy to get wrong.

## Interaction with OR-Tools

The solver side has its own sensitivities. Keep these in mind whenever matrix content or layout changes.

- The solver wants a dense, indexed structure. Indices are positional. If the ordering of locations, legs or vehicles changes in the matrix, every consumer that holds an index from before the change now points at the wrong thing. Reordering is a breaking change even though no value changed.
- Callbacks that read from the matrix are called very often during search. Anything slow in the lookup, such as a network call, a conversion, or a dictionary rebuild, multiplies. Keep lookups cheap and precomputed. Do not add lazy loading inside the callback.
- Search behavior is sensitive to cost structure. A change that makes many legs equal in cost can make search slow or make it return a different but equally good answer. A change that creates very uneven scales can make the first solution poor and the improvement phase slow. If solve times shift after a matrix change, look at the cost structure before blaming the solver settings.
- Time dimensions and cost are separate in the solver but related in the data. If the matrix feeds both cost and travel time, a change to one without the other makes the plan feasible by one measure and not by the other. Keep them derived from the same source.
- Infeasibility can come from the matrix. If an update removes legs that were previously available, the solver may report no solution for loads that used to work. Treat a rise in unplanned loads as a possible matrix symptom.

## Redis caching and invalidation

The matrix, or parts of it, is cached in Redis so that planning and rebalancing do not recompute from scratch. Caching is where most of the hard-to-see failures live.

- Cache keys must reflect everything the value depends on. If you add an input to the cost calculation, such as a new surcharge source or a new carrier attribute, and do not add it to the key or to the invalidation rule, stale values keep being served. The symptoms look like nothing happened.
- Format changes need a plan. If the stored layout of entries changes, old entries and new code meet. Either version the layout in the key, or flush deliberately, and think about what happens to a plan in progress when the flush occurs. Do not rely on expiry to clean up in time.
- Partial updates are risky. Updating some entries while others are still old gives a matrix that mixes two states of the world. Within one planning run the solver should see one consistent snapshot. If readers can see half an update, a route may be costed using a mix of old and new legs and come out cheaper or dearer than either state would give.
- Eviction and memory pressure matter. If entries are evicted under pressure, the code needs a correct fallback that recomputes, and that fallback must produce the same values as the cached path. Test the cold path, not just the warm one.
- Concurrent writers happen. A rebuild and an incremental update can overlap. Use whatever atomic swap or versioning the surrounding code uses, and do not write entries one at a time into the live keys if readers may be active.
- Expiry settings are tuning, not truth. Do not treat a time-to-live as a guarantee that data is fresh. Anything correctness-critical should be invalidated by an event or a version change.

When reviewing a change, ask: what writes this, what reads this, and what happens to a reader that arrives in the middle.

## Delay events and rebalancing

Rebalancing is the reason this component gets touched often. When a delay arrives, costs on affected legs rise, and the rebalancer re-plans some loads. The matrix is what carries the delay into the solver.

- A delay adjustment must be reversible. When the delay clears, the cost has to return to its prior state. If adjustments are applied on top of the current value without recording the base, repeated delays and clearances drift the cost away from the real value over time. Keep the base and the adjustment separate, and derive the effective cost.
- Duplicate and out-of-order events are normal on a message bus. Applying the same delay twice must not double the effect. Applying a clearance before the delay it clears must not leave a leftover. Make adjustments idempotent, keyed by what they describe.
- Rebalancing should not thrash. If a small cost change flips many loads between routes, and the next small change flips them back, dispatchers see loads bouncing. Anything that makes costs jumpy, such as removing smoothing or lowering a threshold, raises the risk. Consider switching costs for moving an already committed load when you work on this area, and do not remove existing ones without checking what they were protecting.
- Loads already in motion cannot be re-planned as freely as loads not yet dispatched. The matrix does not know which loads are committed. If a cost change makes a committed leg look bad, the rebalancer may propose something physically impractical. Check that the consumers filter correctly.
- The cost of delay is not only the matrix entry. Downstream legs shift in time too, and knock-on effects may or may not be modeled. Do not assume a single entry change covers a cascade.

## Pub/Sub inputs and ordering

Updates reach leg-cost-matrix through Google Cloud Pub/Sub, directly or via the services that consume it. Treat the feed as at-least-once and unordered unless the code proves otherwise.

- Handlers must tolerate redelivery. A change that makes a handler non-idempotent will pass tests with single delivery and fail in production on the first redelivery.
- Acknowledgement timing matters. If a handler acknowledges before the matrix write is durable, a crash loses the update. If it acknowledges after a long computation, the message can be redelivered while the first attempt is still running. Keep the work inside the handler short, or extend deadlines in the way the code already does.
- Message schema changes need both sides. A new field the publisher adds will be ignored by an old consumer, and a field the consumer now requires will be missing from older messages still in flight. Make new fields optional with safe defaults on the consumer first, and publish them later.
- Backlogs happen. After an outage, a pile of old updates may be processed in a burst. If handlers apply updates in arrival order without checking how old they are, an old value can overwrite a newer one. Carry some notion of version or time with each update and compare.
- A burst of updates should not trigger a rebuild for each one. Coalesce when possible. Rebuilding on every message is a good way to saturate Redis and the solver at the worst moment.

## FastAPI surface

Some of the matrix is visible through FastAPI endpoints, either directly or through plan and rebalance responses.

- Response shapes are relied on by clients, including dispatcher tooling and other services. Renaming a field, changing its type, or changing what a missing value means is a breaking change even if internal tests pass.
- Request handlers should not block on a full matrix rebuild. If a path ends up triggering a heavy rebuild inside a request, latency spikes and workers pile up. Keep heavy work off the request path.
- Validation errors should stay informative to callers. When adding a new input that affects cost, validate it at the edge, not deep in the build where the failure shows up as an unrelated solver problem.
- Sync and async mixing is a known hazard. Calling blocking Redis or CPU-heavy code from an async handler stalls the event loop for everyone. Follow whatever pattern the nearby handlers use to move such work off the loop.
- Anything that exposes costs externally can leak commercial information, for example carrier rates. Think about who can read an endpoint before adding fields.

## Data sources and the nightly sync

Carrier rates, surcharges and schedule data arrive from outside and land in the matrix through ingestion jobs. The overnight refresh is covered in [[carrier-sync-typical-nightly]]; read that before changing anything about when or how bulk data enters.

- Bad upstream data goes straight into costs. Zero, negative, missing, or absurdly large rates will not be rejected by the solver; it will plan around them. If you touch ingestion, keep sanity checks and make failures loud rather than silently substituting defaults.
- Defaults are decisions. When a rate is missing and the code fills a default, that default affects route choice for those legs. Changing a default is a business-visible change.
- Bulk refresh and live delay updates can collide. A refresh that overwrites the matrix wholesale can erase live delay adjustments that arrived during the refresh. If you change refresh timing or granularity, check how live adjustments are kept or reapplied.
- Time zones and calendar boundaries are a persistent source of off-by-a-period errors in schedule-based costs, especially around daylight saving changes and week boundaries. Do not assume local time in stored data unless the code says so.
- Source identifiers differ between systems. Location and carrier keys may not match across feeds. A mapping change on one side breaks joins on the other, and unmatched rows tend to be dropped quietly. Watch counts of dropped rows after any change here.

## Testing and verification habits

Tests for this component pass too easily. A few habits help.

- Unit tests on small matrices do not show scale problems. Run something close to a real network size at least once for any change to build, layout, or caching.
- Compare before and after on the same inputs. For any cost-affecting change, produce routes under old and new code for a realistic set of loads and read the differences by hand. A few diffs read by a person find more than many assertions.
- Test the cold start, the warm path, and the mid-update path. Many bugs only show up in one of them.
- Test redelivered and reordered events, not only clean ones.
- Check determinism. The same inputs should give the same matrix. Iteration order of sets and dictionaries, parallel builds, and floating point accumulation order can all introduce run-to-run differences that make debugging miserable and cause plan flicker.
- Keep fixtures current. Old fixtures may encode assumptions, such as symmetry or a particular scale, that the code no longer holds, so a passing test may be testing a fiction.
- When a change is meant to be behavior-neutral, prove it with a diff of the matrix itself, not by inference from code reading.

## Rollout and operations

- Prefer changes that can be turned off. A new cost ingredient behind a switch can be backed out without a deploy if dispatchers complain.
- Roll cost-model changes out when dispatch volume is low and someone is watching. A change to how costs are computed lands as changed routes for real loads.
- Warn dispatchers or whoever supports them about expected route differences. A route that changes with no explanation looks like a bug even when it is an improvement.
- Keep an eye on solve time, unplanned load count, rebalance frequency, cache hit rate and Redis memory after a change. Any of them moving is a hint.
- Rolling back code does not roll back cached data. If the new code wrote entries in a new form or with new values, clear or version them as part of the rollback, or the old code will read them.
- Mixed versions run side by side during a deploy. For a while, old and new instances both read and write the same cache and the same feed. Changes must be safe in that window.

## Small things that keep biting

- Searching for a name finds the obvious callers but not those that build keys from strings. Grep for the string forms as well.
- Comments in this area are sometimes out of date. Trust the code and the data over a comment about units or direction.
- Helper functions that look generic often carry an assumption about mode or direction. Read them before reusing them for a new case.
- Logging every entry on a hot path floods logs and slows things down. Log summaries.
- Do not fix a surprising cost by special-casing one pair of locations. If one pair is wrong, others are probably wrong for the same reason.
- If a change seems to need edits in the build path, the update path, the cache layer and the event handlers all at once, stop and check whether you are changing the contract. If so, say so in the change description so reviewers look at the consumers and not just the diff.
- When in doubt about what a number means, find where it is consumed and follow it to the solver before changing how it is produced.
