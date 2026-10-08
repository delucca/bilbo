---
id: 01KRKR14Q1WP8TRK8MWRY5066D
created: 2026-05-14T14:19-03:00
---

# store-features-delta history retention

This note replaces the earlier note about "store features delta table 2". The history retention of store-features-delta is now `interval 60 days`.

## Summary of the change

The retention used to be shorter. It is now `interval 60 days`. The reason is the two-month seasonal comparison window: analysts compare current store-level behaviour against the same stretch of the prior period, and the older, shorter retention cut that history off before the comparison could be made.

## Why this value

Merchandising analysts at grocery chains look at seasonal shifts over about two months. The retained history has to be at least as long as that window, or the comparison reads versions that were already cleaned up. `interval 60 days` covers the window. It is not padded beyond that, because longer history means more storage and slower file listing.

## What the retention governs

It governs how long old versions of the store-features-delta table stay available for time travel. Older table versions and their data files remain readable inside the window. Past it, they become eligible for cleanup.

## What it does not govern

It does not change the current state of the table. Current rows, schema and partitioning are unaffected. It also does not change how the feature job writes. Only the length of kept history changes.

## Where it applies

The setting lives on the Delta table that holds store-level features for ShelfSense. It is a table property, not a Spark session setting, so it holds for every job and every reader that touches store-features-delta, whichever cluster they run on.

## How the property is set

Delta table properties are set with an ALTER TABLE statement. The value written is the interval string exactly as recorded here.

```sql
ALTER TABLE store_features_delta
SET TBLPROPERTIES (
  'delta.logRetentionDuration' = 'interval 60 days'
);
```

Check the real table name in the catalog before running it. The name above is a stand-in for the component, not a verified identifier.

## Log retention versus file retention

Delta has two related settings. Log retention decides how long the transaction log entries are kept. Deleted data file retention decides how long removed files are kept before cleanup removes them. Time travel needs both to reach back far enough. If only the log setting is raised and the file setting stays short, old versions may exist in the log but their files can be gone after a cleanup run.

## Cleanup jobs

Any scheduled cleanup (vacuum) on store-features-delta must not remove files that are still inside the window. Check that the file retention in the vacuum call is not shorter than the log retention. If it is, the longer log retention gives a false promise.

## Airflow

The Airflow DAGs that refresh store-features-delta and run maintenance should be read once after this change. Look for any step that passes its own retention value to vacuum. A hard-coded shorter value there overrides the intent of this note.

## Spark jobs

Scala Spark jobs that read earlier versions by version number or timestamp now have a 60-day horizon. Anything asking for an older version will fail, as before, but the cutoff has moved.

## Snowflake

Downstream Snowflake tables are loaded from the features and keep their own history. This retention does not change anything on the Snowflake side. If analysts need the comparison window there, that is a separate setting.

## Storage cost

Keeping more history means more files stay on storage. The growth is bounded by the window, so it levels off after the window fills. Watch storage for the first two months after the change and compare it with the earlier level.

## Rollout

The property takes effect on the table once set. The window does not fill instantly: history already deleted under the older retention does not come back. Full two-month comparison is only available after the table has accumulated that much history under the new setting.

## Known limits

Versions removed before the change are gone. Do not promise analysts a comparison that reaches past what was actually retained. Tell them the date from which the full window is available.

## Verification

After setting the property, read the table properties and confirm `interval 60 days` shows as the log retention. Then read an old version near the edge of the window to confirm it loads. Run both checks on the real table, not a copy.

## Open items

- Confirm the file retention used by the vacuum step matches the log retention.
- Confirm no Airflow task overrides the property.
- Tell the analysts when the full comparison window is available.

## Rollback

If storage cost becomes a problem, the property can be set back to a shorter value with the same kind of statement. That would again cut the seasonal comparison, so talk to the analysts first.
