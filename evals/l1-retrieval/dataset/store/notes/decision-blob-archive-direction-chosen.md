---
id: 01KZJDV3758SR8FZ6EWB9MKP4T
created: 2026-08-09T01:52-03:00
---

# blob-archive-client: general direction

We settled on a general direction for blob-archive-client and this note keeps it so nobody has to argue it again. It is the thin layer that moves instrument output and notebook attachments into Azure Blob Storage. It stays small, and it does not own business rules. Those live in the sync service that calls it.

## Why this note exists

The component had grown a few habits that nobody chose on purpose. Retries were scattered, naming of stored objects differed between callers, and some callers talked to Blob Storage directly. The team agreed to pull that back into one client with a clear job.

## Scope of the client

blob-archive-client writes, reads and verifies archived objects. It does not decide what to archive. It does not touch the SQL Server audit tables. It does not consume RabbitMQ messages itself; the worker that does so calls the client.

## Immutability first

Archived content is treated as write-once. The client should make overwriting an existing object hard to do by accident. If a correction is needed, it is stored as a new object that points back at the old one. Compliance officers need the original to stay readable, so this is not up for trading against convenience.

## Integrity checks

Every upload is checked against a content hash that the client computes before sending. Every download is checked again on the way out. A mismatch is surfaced as a failure the caller must handle, never silently retried into success. The hash is also handed back so the caller can record it next to the audit entry.

## Naming of stored objects

Object names come from one function in the client. Callers pass in the logical entry and the instrument run, and the client builds the name. Callers must not build names by hand. Names carry no personal data and nothing a scientist typed free-form.

## Retries and failures

Transient storage errors get a bounded retry with backoff, in one place. Anything that is not transient fails fast with a clear exception type. The client does not swallow errors to keep a queue moving. If it cannot confirm a write, the caller is told the write is unconfirmed, and the audit trail must not claim it happened.

## Idempotency

Messages from RabbitMQ can arrive more than once. The client is built so that repeating the same upload for the same content is safe and gives the same result. This is what lets the worker redeliver without us writing special cases.

## Auth and configuration

Access uses managed identity where the host supports it, and connection secrets stay out of source and logs. Configuration is injected, not read from globals, so tests can swap in a fake store.

## Logging and audit

The client logs what it did and to which logical entry, never the content. Log lines should be enough to line up with an audit record later. The authoritative audit trail stays in SQL Server, not in client logs.

## Testing approach

Unit tests run against an in-memory fake of the storage interface. A smaller set of integration tests runs against the Azure storage emulator. We do not test against the real account from CI.

```csharp
public interface IBlobArchiveClient
{
    Task<ArchiveReceipt> PutAsync(ArchiveRequest request, CancellationToken ct);
    Task<Stream> GetAsync(ArchiveReference reference, CancellationToken ct);
}
```

The shape above is the idea, not a frozen signature. Keep the surface this small.

## What we are not doing

- No direct Blob Storage calls from other projects.
- No delete operation on the public interface.
- No caching layer inside the client for now.
- No format conversion; bytes in, same bytes out.

## Open questions

Lifecycle and cost tiers for old objects are not settled and need input from compliance. How long things must be kept is their call, not ours. How we handle very large instrument files, streaming versus chunked, still needs a look.

## Next steps

Move existing direct callers over to the client one at a time. Pull the naming and retry code into the client first, since the rest depends on it. Add the integrity check to the read path before touching anything else.
