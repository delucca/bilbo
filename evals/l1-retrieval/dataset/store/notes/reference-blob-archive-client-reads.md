---
id: 01KX3Q17NDZ0QY6JHDWG90GGVJ
created: 2026-07-09T12:13-03:00
sources:
  - "code: src/BlobArchive/BlobClientFactory.cs"
---

# blob-archive-client reference

blob-archive-client is the part of LabNotebook Sync that talks to Azure Blob Storage. It stores archived notebook material and instrument output so the audit trail has something durable to point at. This note is a quick reference: where its configuration comes from, what it used to be called, and what to check when it misbehaves. Details I did not verify are kept general on purpose.

## What it is

blob-archive-client is a C# library inside the .NET solution. Other components hand it content to archive and ask it to fetch content back. It does not own notebook entries or the audit records themselves. Those live in SQL Server. RabbitMQ carries the messages that tell the archive side when something new needs to be written. The client only deals with blobs.

Research scientists never see it directly. Compliance officers see it indirectly, because an audit trail entry that references archived data depends on this client having written that data correctly.

## Former name: vaultclient

The old name of this component was `vaultclient`. It is now called `blob-archive-client`. If you find `vaultclient` in an old branch, a ticket, a dashboard, a log line, a runbook or a script, it means the same thing as `blob-archive-client`. Use `blob-archive-client` in new code, docs and notes.

When searching the history, search for both names. Older commits and older discussion threads use `vaultclient` only, so a search for the new name alone will miss them.

Do not assume that every identifier was renamed along with the component. Some leftovers under the old name may still exist in places that were never touched. Treat each one as a candidate cleanup, not as a separate component.

## Configuration: the connection string

The storage connection string is read from the environment variable `LNB_BLOB_CONNECTION`. The wiring is in `src/BlobArchive/BlobClientFactory.cs`, which is where the client gets built from that variable.

Practical consequences:

- If `LNB_BLOB_CONNECTION` is not set in the process environment, the client cannot be created. Check the environment of the actual running process, not just your shell.
- The value is a secret. Do not paste it into tickets, logs or notes.
- Anyone changing how the client authenticates to storage should start in `src/BlobArchive/BlobClientFactory.cs` and keep the variable name stable, or update every deployment that sets it.
- The old name `vaultclient` does not change the variable name. Nothing here should be looked up under a variable named after the old component.

## Where to look in the code

Start with `src/BlobArchive/BlobClientFactory.cs`. It is the single place where configuration turns into a storage client, so any question about which account or container the client talks to ends up there. Everything else in the component takes the already built client.

If a service seems to write to the wrong storage account, the cause is almost always the environment variable of that service, not logic in the client.

## Failure modes to check first

When archiving fails, work through these in order:

- The connection variable is missing or empty in the process that failed.
- The connection string points at the wrong environment, for example a test account from a production host.
- The network path from the host to Azure Blob Storage is blocked or the credentials expired.
- A message was consumed from RabbitMQ but the blob write failed, leaving the audit trail pointing at nothing. This is the case compliance cares about most.

For the last one, compare what the SQL Server audit records reference with what is actually in storage before retrying anything.

## Audit trail concerns

The audit trail must not reference blobs that do not exist, and archived blobs should not be silently replaced. Any change to how the client writes, overwrites or deletes content needs a compliance review, not only a code review. Keep that in mind before adding convenience features such as automatic overwrite on conflict.

## Open items

- Sweep the repo and docs for remaining uses of `vaultclient` and rename or annotate them.
- Write down, next to the factory, which environments set `LNB_BLOB_CONNECTION` and who owns rotating it.
- Add the verified list of failure messages to this note when someone has seen them in practice. None are recorded here yet.
