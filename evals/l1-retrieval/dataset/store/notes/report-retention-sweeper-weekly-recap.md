---
id: 01K1XMYR0E0CE5PHRFSJGCBBHB
created: 2025-08-05T14:37-03:00
---

# retention-sweeper weekly recap

Short recap of the week on retention-sweeper. Most of the time went into reading how it decides what is eligible for removal, and into making the audit side of that decision easier to follow. Nothing here is a final call; it is where things stand so the next session does not start cold.

## Where things stand

retention-sweeper is a background worker in the C# service set. It walks notebook entries and the instrument output attached to them, checks each item against the retention rules, and removes or archives what has aged out. The audit trail is the part compliance cares about most, so every removal has to leave a record that outlives the data.

The week was mostly cleanup and investigation rather than new features. The sweep logic itself is mostly unchanged.

## Eligibility logic

I read through the code that decides whether an item can go. The rules are layered: a base retention period, then holds that override it. A hold always wins over age. The confusing part is that holds come from two places, the notebook entry and the instrument run it links to, and they are evaluated in separate steps.

Open question: whether a hold on the linked run should block removal of the entry itself or only of the raw output. Right now it blocks only the raw output. I did not change that. It needs a word from compliance before anyone touches it.

## SQL Server side

Candidate selection is a query against the entry tables. It reads fine, but it pulls candidates in one large pass and then filters in memory. That works for the current data volume and will get slow on larger sites.

I sketched a batched version that filters in the database and pages through results. It is not merged. The main risk is that paging while rows are being deleted can skip rows, so it needs a stable ordering key.

## Blob storage cleanup

Raw instrument files live in Azure Blob Storage. The sweeper deletes the database record and the blob in two separate steps, and a crash between them can leave an orphan blob. I confirmed the order is record first, blob second, which means an orphan is the failure mode and not a dangling record. That is the safer direction for audit purposes.

An orphan reconciliation pass would be a good follow-up. I only wrote down the idea.

## RabbitMQ messages

The sweeper publishes a message for each removal so the audit consumer can write its record. I checked that messages are marked persistent and that the consumer acknowledges only after the audit row is written. Redelivery can produce duplicate audit rows, and I saw no idempotency guard on the consumer side. Worth fixing.

## Audit trail

The audit rows record who or what triggered the removal, the rule that applied, and the item reference. They do not record which hold checks were evaluated and passed. For a dispute that gap matters, because we can show what was removed but not that a hold was considered.

I drafted the extra fields but did not add a migration.

## Tests

Existing tests cover the age rule well and the hold rule thinly. I added a few cases around holds on linked runs to pin down the current behaviour, so a later change to it shows up as a deliberate edit to a test. No test covers the crash-between-steps case for blob deletion. That one needs a fake blob client that fails on demand.

## Local setup notes

Running the sweeper locally needs a SQL Server instance, a broker, and a storage emulator. The emulator is the fiddly one. If blob deletes seem to succeed but nothing changes, check that the emulator is the one the config points to before blaming the code.

A dry-run mode exists, and I used it for most of the investigation. Something like this:

```
dotnet run --project retention-sweeper -- --dry-run
```

I am not certain the flag spelling is stable, so check the entry point before relying on it.

## Next week

- Get an answer from compliance on holds for linked runs.
- Add an idempotency guard to the audit consumer.
- Decide whether to add the evaluated-checks fields to the audit rows.
- Try the batched candidate query against a larger data set.
- Write the failing-blob-client test.
- Think about the orphan reconciliation pass, but only after the items above.

## Loose ends

The batched query branch is half done and should be rebased before anyone builds on it. Nothing from this week has been deployed.
