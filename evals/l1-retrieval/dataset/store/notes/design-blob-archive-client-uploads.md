---
id: 01KCX6YV75M9J4QPPD1KJEYXVG
created: 2025-12-20T03:25-03:00
sources:
  - "code: src/BlobArchive/UploadPipeline.cs"
---

# blob-archive-client design

blob-archive-client is the part of LabNotebook Sync that moves attachment files out of the notebook pipeline and into long-term storage in Azure Blob Storage. Attachments are things like instrument output files, images, exported spreadsheets and PDFs that a scientist links to a notebook entry. This note records how the client is shaped and why, so the next person does not have to rediscover it.

## Purpose and scope

The client takes an attachment that the sync service has already accepted, and stores the bytes in blob storage. It does not decide which entries need syncing, and it does not own the audit trail. It reports what happened and the audit side records it. Keeping those two apart matters for compliance officers, who need the audit record to stand on its own even if the archive has a problem later.

It is written in C# on .NET, like the rest of the sync service. It runs inside the same process family as the other workers and is driven by messages from RabbitMQ, so an upload is always the result of a queued job and never a direct call from a user request.

## Upload layout

blob-archive-client uploads attachments to the Azure Blob Storage container `lnb-attachments` in blocks of `8 MiB`. Each attachment becomes one blob in that container. The file is cut into blocks of that size, each block is staged, and then the full block list is committed so the blob only appears once it is complete. The last block is usually smaller than the rest.

Why blocks: instrument output can be large, and a single long request is fragile on lab networks. Smaller blocks mean a dropped connection costs one block, not the whole file. The size is a compromise between request count and memory held per upload. Do not change it casually, because memory use per worker scales with it and with the number of concurrent uploads.

## Retries and resume

A failed block is retried on its own with backoff. Because blocks are staged before commit, a retry of the whole job can reuse blocks that were already staged when they are still held by the service, and otherwise stages them again. Nothing is visible to readers until commit, so a half-finished upload never looks like a real attachment.

If retries run out, the job is not acknowledged as done. It goes back through the normal RabbitMQ dead-letter path so someone can look at it. The client should never swallow the error and report success.

## Integrity and audit

Before commit the client compares what it sent with what it read from the source file, using a content hash. The hash is also handed to the audit side, so the audit trail can show that the stored blob matches the file the scientist attached. This is the check compliance people actually rely on, so it must not be skipped to save time on big files.

Metadata that links the blob back to the notebook entry, such as the entry reference and the instrument run, is set on the blob at commit. The SQL Server database remains the source of truth for the link. Blob metadata is a convenience for recovery, not the record.

## Gotchas

- Overwriting an existing blob under the same name is not an update path for audited data. Treat stored attachments as immutable; a changed file is a new version with its own blob.
- Do not log file contents or full signed URLs. Logs are read by more people than the data should be.
- Concurrency limits are per worker. Running more workers multiplies memory and storage requests, so check both before scaling out.
- Clock differences between workers and the storage service can confuse timestamps. Use the service-reported commit time where the audit record needs one.

## Open questions

- Whether large files should use a larger block size is untested. Anything that changes it needs a look at worker memory first.
- Lifecycle rules for old attachments are not settled and depend on retention rules from compliance.
- Resume across worker restarts works only as far as staged blocks survive. How long they survive has not been confirmed here, so confirm against the storage documentation before relying on it.
