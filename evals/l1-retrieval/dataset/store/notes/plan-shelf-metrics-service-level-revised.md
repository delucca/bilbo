---
id: 01KQHAEPVXA0DMP2SDH36YGB27
created: 2026-05-01T05:28-03:00
---

# shelf-metrics-lib service_level plan

This note replaces the earlier note on "shelf metrics service level". The new value: the `service_level` metric of shelf-metrics-lib moves to release 1.6.0, replacing the earlier target, because the calendar API change ships first. Anything in the old note that pointed at the earlier target is out of date; use 1.6.0.

## Decision in one place

The `service_level` metric of shelf-metrics-lib is now planned for release 1.6.0. The earlier target is dropped. The reason is ordering: the calendar API change ships first, and the metric depends on it. Shipping the metric before that change would mean computing it against the old calendar behavior and then redoing it. So the metric waits and lands in 1.6.0.

If someone asks "which release carries service_level for shelf-metrics-lib?", the answer is 1.6.0, and the reason is that the calendar API change has to go out first.

## Why the calendar change comes first

service_level is computed over store-days. Which days count as open, which are holidays, and how week boundaries fall all come from the calendar API. The calendar change alters how those days are resolved. If service_level is built on the old resolution, the numbers shift once the calendar change lands, and merchandising analysts would see a metric that moves for no business reason.

Doing the calendar change first keeps one definition of a store-day in the library and lets us write the metric once, against the final behavior.

## What to do, in order

1. Confirm the calendar API change is merged and released in shelf-metrics-lib before touching service_level.
2. Rebase the service_level work onto that change. Do not carry over assumptions from the old branch about day boundaries.
3. Re-check the Spark jobs that call the metric. They read from Delta Lake tables and the calendar join is where the change bites.
4. Re-run the comparison against the existing Snowflake reporting tables so analysts can see the old and new values side by side.
5. Cut the release as 1.6.0 and update the Airflow DAGs that pin the library version.

## Risks and things to watch

- Downstream consumers that pinned an earlier version will not see the metric until they bump to 1.6.0. Tell the owners of the replenishment order generation first, since it consumes store-level service figures.
- Airflow DAGs that install the library per run may pick up the new version at different times. Check each one rather than assuming a single bump covers all.
- Backfills: historical service_level values computed before the calendar change will not match values computed after. Decide per table whether to backfill or to mark a cutover date. This is not settled yet.
- Scala API compatibility: if the calendar change breaks signatures, the metric code must follow the new ones, not shims.

## Open items

- Who signs off the 1.6.0 release notes. Not decided.
- Whether analysts need a short note explaining the cutover in the metric's values.
- Whether the old note should be deleted or kept as history. This note supersedes it either way, so do not follow its target.

## Status

Planned, not started. Blocked only on the calendar API change shipping. Once that is out, the work on the metric can proceed straight to the 1.6.0 release.
