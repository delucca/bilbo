---
id: 01KW5N48CZ8M6AV9YTDRRM6SB5
created: 2026-06-27T20:02-03:00
sources:
  - "code: src/EntrySync/appsettings.json"
---

# entry-sync-service batch size setting

Quick reference for how entry-sync-service decides how many notebook entries it handles in one pass. Read this before touching throughput or memory behaviour.

## Where the setting lives

The batch size of entry-sync-service is set by the key `Sync:BatchSize` in `src/EntrySync/appsettings.json`. The value there is 100 entries. That is the only place the default is defined, so if you see a different number at runtime, something has overridden it.

## What the value means

One batch is the group of lab notebook entries that entry-sync-service picks up, processes and acknowledges together. With the shipped value, each pass covers 100 entries.

## Example

```json
{
  "Sync": {
    "BatchSize": 100
  }
}
```

## Reading it in code

The key is read through the normal .NET configuration binding, so the section name is `Sync` and the property is `BatchSize`. The colon form `Sync:BatchSize` is what you use for environment overrides and for lookups by key.

## Overrides

Standard .NET configuration layering applies. Environment variables and other providers can replace the file value. When debugging, check the effective configuration, not only the file.

## Why it matters for SQL Server

Each batch touches SQL Server for reads and writes of entry state. A larger batch means fewer round trips but longer transactions and more locking. A smaller one is gentler on the database.

## Why it matters for RabbitMQ

Messages are consumed from RabbitMQ and grouped by the batch size. Larger batches hold more unacknowledged messages at once, which raises redelivery volume if the service dies mid-batch.

## Instrument output and Blob Storage

Instrument output attached to entries may be fetched from Azure Blob Storage during a batch. Big batches with large attachments increase memory use and can stretch processing time.

## Audit trail considerations

Compliance officers rely on the audit trail being complete for every entry. Batch size should never change what gets recorded, only how entries are grouped. If a change to the value seems to alter audit output, treat that as a bug.

## Changing the value

Edit the key in the file, redeploy, and watch the first few batches. Do not set it to zero or a negative number.

## Tuning notes

Lower it if you see timeouts or lock contention on the database. Raise it only after confirming the queue is backing up and the database has headroom.

## Testing

Tests that depend on batch boundaries should set the value explicitly instead of relying on the default, so a later change to the default does not break them.

## Common mistakes

- Editing a different appsettings file than the one under `src/EntrySync/`.
- Forgetting an environment override that hides the file value.
- Assuming the batch size caps queue depth. It does not.

## Open questions

Nobody has measured the best value for large instrument attachments. Worth a benchmark before changing the default.
