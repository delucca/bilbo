---
id: 01KSVABYGTZRA23MYXGFRMTCD0
created: 2026-05-29T23:10-03:00
---

# results-ledger retention spec

This note replaces the earlier note about "results ledger rows must" be kept; the new retention value is 730 days, up from the earlier 400 days. Anything that still quotes the old figure for results-ledger is out of date and should be fixed to match this note.

## Retention rule

results-ledger rows must be retained for 730 days. The clock runs per row, from the moment the row was written, not from the end of the event or the poll it belongs to. A row is not eligible for removal until it has been in the ledger for the full 730 days. After that it may be purged by the normal cleanup job, but nothing requires it to go right away.

The old rule kept rows for 400 days. That is no longer the rule. Rows that were written under the old rule and are still present stay; they are now covered by the longer window and must not be purged early just because they passed the old cutoff.

## What this covers

The rule applies to every row in results-ledger, whatever the type: final poll tallies, intermediate snapshots, Q&A vote counts, and the records of moderation actions that changed a result. If a row is part of the ledger, it follows the same window. There are no per-event overrides and no shorter window for small events.

It does not change how long live data stays in the real-time path. WebSocket session state and in-memory counters in the Phoenix layer keep their own lifetimes. Only the durable ledger in CockroachDB is governed here.

## Why it changed

Event producers and community managers asked for a longer window so they can answer questions about past events, including disputes over a result and year-over-year comparisons. The old window cut off rows too early for that. The longer window covers a full year of reporting plus a second year of lookback for disputes.

## Impact on implementation

- Purge or TTL logic for results-ledger must use the new value. Check any place where the retention is set in config, migrations, scheduled jobs or documentation, and update all of them together.
- Storage will grow. Rows stay almost twice as long as before, so capacity planning and cost estimates for CockroachDB need to account for it. Watch table size and range counts after the change.
- Deletes should stay batched and off-peak, as before, so a purge never competes with a large live event.
- The Next.js dashboards that show history can now show older results. Any UI text that mentions the old retention needs to be updated.

## Open points

- Whether older rows should move to cheaper storage later is undecided. For now everything stays in the main ledger.
- Export and deletion requests from users are handled separately and are not changed by this rule. If a legitimate deletion request applies to a row, it takes precedence over the retention window.
