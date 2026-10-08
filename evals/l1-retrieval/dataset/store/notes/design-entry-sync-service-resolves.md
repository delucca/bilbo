---
id: 01JSA1Q4ZEYVWEZVHED1KXZ1V3
created: 2025-04-20T13:50-03:00
---

# entry-sync-service design

entry-sync-service is the part of LabNotebook Sync that moves electronic lab notebook entries between clients, the database and instrument output, and keeps the audit trail intact. Its internal codename is `quillbridge`. You will see that name in older tickets, repo folders and chat, and it means the same thing as `entry-sync-service`. This note covers how it is put together and, mainly, how it settles conflicting edits.

## Naming

The codename `quillbridge` is internal only. Documentation and anything user facing should say `entry-sync-service`. If a search for one name finds nothing, try the other.

## Purpose

Scientists edit entries from several places, and instruments push results in at the same time. The service has to merge all of that into one stored version per entry. Compliance officers need to see who changed what and when, so no change may vanish without a trace.

## Main pieces

The service is a C# worker on .NET. It reads and writes entries in SQL Server, takes work from RabbitMQ queues, and stores large instrument files in Azure Blob Storage. The database holds only references to those blobs.

## Conflict resolution

Conflicting edits are resolved with last-writer-wins. The comparison uses the `revision_vector` column of each entry. When two edits touch the same entry, the service compares their `revision_vector` values and the later one becomes the current version. It does not try to merge text from both edits.

## What the losing edit becomes

The losing edit is not thrown away silently. It is kept in the audit history, marked as superseded, so a reviewer can still read it. Last-writer-wins only decides which version is current. It does not decide what is remembered.

## Why last-writer-wins

A field-level merge of scientific text is risky. It can produce a sentence nobody wrote, which is a problem in a regulated record. A simple rule that everyone can predict was preferred, with the audit trail covering the loss.

## Messaging

Edits and instrument results arrive as RabbitMQ messages. Handlers should be safe to run twice, since a message may be delivered more than once. Comparing `revision_vector` helps here: a replayed older edit loses and changes nothing.

## Instrument output

Instrument files go to Azure Blob Storage first. The entry then gets a reference and a new revision. Large payloads never pass through the database.

## Audit trail

Every accepted or superseded change writes an audit record in SQL Server. These records are append only. The service never updates or deletes them.

## Known tradeoffs

A user whose edit loses may be surprised. The client should tell them that a newer version exists and show it. Clock drift is not a concern for ordering, because the comparison uses `revision_vector` and not wall clock time.

## Things to watch

Do not change how `revision_vector` is written without checking every writer. Instrument ingestion and client edits both update it. A path that forgets to bump it will lose conflicts it should win.

## Open questions

Whether some entry types, such as signed or locked entries, should reject edits instead of competing is still undecided. Compliance input is needed.

## Related notes

Look for notes on the audit trail schema and on queue handling if you work on this component.
