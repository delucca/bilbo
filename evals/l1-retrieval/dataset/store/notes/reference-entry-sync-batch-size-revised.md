---
id: 01M3EB88748N8PMYJNCEMWXK04
created: 2026-09-26T04:53-03:00
---

# entry-sync-service batch size

This note replaces the earlier note about "entry sync batch size". The current value is 250 entries for `Sync:BatchSize` in entry-sync-service.

## Current value

The `Sync:BatchSize` key of entry-sync-service is now `250` entries. The earlier value was 100, and that value no longer applies. Anyone reading older notes or configs that mention 100 should treat them as out of date.

## What the key controls

`Sync:BatchSize` sets how many lab notebook entries entry-sync-service handles in one batch when it syncs entries with instrument output. With the new setting, each batch carries 250 entries instead of the former 100.

## Where it lives

The key is part of the configuration of entry-sync-service, which is a C# / .NET service. It is read as a normal configuration setting, so it can be set in the service's settings files or overridden per environment in the usual way. Check the deployed environment's configuration rather than assuming it matches the default.

## Related pieces

entry-sync-service talks to SQL Server for entry data and the audit trail, to RabbitMQ for messages about instrument output, and to Azure Blob Storage for larger instrument files. A batch size change touches how much work is pulled and written per cycle across those pieces.

## Audit trail

The audit trail still has to be complete whatever the batch size. Every entry in a batch must produce its own audit record, so compliance officers see the same detail as before. Larger batches do not change that requirement.

## Things to watch after the change

Look at the time one batch takes, the size of each SQL Server transaction, and the number of messages waiting in RabbitMQ. If any of these look worse than before the change, compare against the previous setting of 100 entries.

## Rolling back

To go back, set `Sync:BatchSize` to the old value of 100 in the configuration and restart entry-sync-service. Note that this reverts to the superseded value, so record it in this note if it is ever done.

## Open items

No other settings of entry-sync-service were changed along with this one. If other keys change later, add them here instead of starting a new note.

## Quick reference

Component: entry-sync-service. Key: `Sync:BatchSize`. New value: `250`. Old value: 100, superseded.
