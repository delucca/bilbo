---
id: 01KPP53TGZ6B2Z3FFCHWDYTZ5E
created: 2026-04-20T16:15-03:00
---

# retention-sweeper spec

This note specs what retention-sweeper does and, above all, what it must never do. The rule that outranks everything else: retention-sweeper must never delete an entry whose `LegalHold` column is set to 1. That holds for every entry, every code path, every run mode, and every actor. It holds even if the retention period has passed, an operator asks for the delete, or a cleanup job is already half done. If `LegalHold` is 1, the entry stays, and so do its attachments, its instrument output and its audit history.

Written in a hurry, so it is plain. The people who read it are research scientists, who mostly care that their data is not lost early, and compliance officers, who care that nothing is lost when it is under hold. When those two goals conflict, the compliance officer wins.

## Purpose and scope

LabNotebook Sync joins electronic lab notebook entries to the output of lab instruments and keeps an audit trail of every change. Over time this piles up: entries, revisions, instrument files, derived artifacts, and audit rows. Some of it has to be kept for a set period. Some of it should be removed once that period is over, because storage costs money and because some data-protection rules require removal. retention-sweeper is the background component that finds data past its retention period and removes it.

retention-sweeper is a sweeper, not a policy author. It does not decide how long anything is kept. It reads the retention rules that compliance officers maintain elsewhere, applies them, and records what it did. If the rules are missing, unreadable, or contradict each other, it does nothing for the affected entries and says so. Doing nothing is always the safe fallback. Deleting is the risky action, so it needs positive evidence at every step.

In scope:

- Finding entries that are eligible for removal under the current retention rules.
- Checking each candidate against hold status before touching anything.
- Removing the entry rows from SQL Server and the matching binary payloads from Azure Blob Storage.
- Writing audit records about what was removed, what was skipped, and why.
- Publishing events on RabbitMQ so other components learn about removals.

Out of scope:

- Setting or clearing a legal hold. That is done by compliance tooling, not by retention-sweeper. The sweeper only reads `LegalHold`.
- Editing retention rules.
- Restoring deleted data. Once removed, it is gone, apart from whatever the storage layer's own recovery features give.
- Pruning the audit trail itself in a way that loses the record of a removal. The audit entry saying that something was removed outlives the thing removed.

## The LegalHold rule

The column is `LegalHold`. Value 1 means the entry is under hold. The sweeper treats any entry with `LegalHold` set to 1 as untouchable.

What "untouchable" covers, in detail:

- The entry row itself is never deleted, and never soft-deleted or marked as expired in a way that makes another process delete it later.
- Every revision of the entry is kept. A hold on an entry covers its whole history.
- Instrument output files linked to the entry in Azure Blob Storage are kept. Skipping the row but deleting the blobs would be just as bad as deleting the row, and it is the easier mistake to make because the blob cleanup is a separate step.
- Derived artifacts built from the entry, such as rendered copies or exports, are kept as long as the entry is.
- The audit rows for the entry are kept, as they would be anyway.

What the sweeper must do when it meets a held entry: leave everything alone, write an audit record that the entry was skipped because of the hold, and carry on. Skipping is a normal outcome, not an error. It must not fail the run, and it must not retry the entry in a tight loop.

What the sweeper must do when it cannot tell whether an entry is held: treat it as held. This includes a null or unexpected value in `LegalHold`, a failed read, a timeout, or a row that changed between the check and the delete. The guard is conservative on purpose. The only state that allows deletion is a positively read value that means "not held", read inside the same transaction that does the delete.

Do not rely on a hold check done earlier, for example when the candidate list was built. A hold can be placed at any time, including a moment after a candidate query returns. So the check is repeated at delete time, under a lock or in a transaction that stops the value changing between the check and the delete. The delete statement should itself carry the hold condition, so that even if someone later adds a code path that forgets the earlier check, the database refuses to remove a held row. Belt and braces here is intended, not clutter.

A hold also overrides things that would normally force a deletion. Retention expiry does not outrank it. A user request for erasure does not outrank it, though the request is recorded and handed to the compliance team to deal with. An operator running the sweeper by hand with a force option does not outrank it. If someone proposes adding a force option that bypasses `LegalHold`, the answer is no. That would be a change to this spec and needs a compliance officer's sign-off, not just a code review.

## How a sweep runs

A sweep is a pass over the data that applies retention rules and removes what is eligible. It runs on a schedule and can be started on demand. The behavior is the same either way.

The steps, in order:

- Load the current retention rules. If they cannot be loaded, stop the sweep and report. Do not use stale rules from memory without noting it; a rule that was tightened or relaxed since could lead to a wrong delete.
- Build the candidate list in bounded batches. Batching keeps transactions short and keeps the sweeper from hurting the interactive users who share the SQL Server database. A candidate is an entry whose retention period has ended under the rules.
- For each candidate, begin a transaction and re-read the entry, including `LegalHold`. If `LegalHold` is 1, or its state is unknown, skip it, record the skip, and move on.
- If the entry is eligible and not held, remove the database rows in the transaction. Record the intent to remove the blobs in a durable way before committing, so a crash between the two does not leave orphans that nobody knows about.
- Commit, then remove the blobs from Azure Blob Storage. If blob removal fails, the row is already gone, but the recorded intent means a later pass retries the blob removal. Blob cleanup is retried by id, and it must also re-check that no hold applies to anything the blob still belongs to, in case a blob is shared.
- Write the audit record for the removal and publish an event to RabbitMQ.
- Continue to the next candidate. At the end of the sweep, write a summary.

Ordering matters. Rows first, blobs after, intent recorded before commit. The reverse order is the dangerous one: deleting blobs first and then failing to delete the row leaves an entry that points at nothing, which looks like data loss to the scientist and like tampering to an auditor.

A sweep should be safe to interrupt. Stopping it in the middle, whether by deploy, crash or operator, must leave the data consistent. Every candidate is processed in its own transaction, so an interruption costs at most the candidate being worked on, and that one either committed fully or not at all. The next sweep picks up whatever is left.

A sweep should be safe to run twice. Two sweepers running at once would race on the same candidates. Use a lease or a similar single-runner guard so only one sweep is active. If the guard cannot be taken, the second one exits without doing anything and says why. Even with the guard, each delete still carries its own hold condition, so a guard failure cannot turn into a wrongful delete.

## Auditing and events

The product exists to enforce audit trails, so retention-sweeper has to be as visible as any user. Every decision it makes leaves a record, including the decision to do nothing.

For each removal the audit record states which entry was removed, which retention rule applied, when, and that the actor was retention-sweeper and not a person. For each skip it states which entry, and the reason: held, rule unclear, state unknown, or not yet due. Skips for holds are the ones compliance officers will ask about, so they must be easy to find and not buried among routine skips.

Audit writes and deletes belong in the same transaction where possible. If the audit write fails, the delete must not go ahead. An unaudited deletion is worse than a delayed one. This is the one place where being strict makes the sweeper slower, and that is accepted.

The summary at the end of each sweep gives counts of removed, skipped for hold, skipped for other reasons, and failed. A rise in held skips is normal if a new matter has begun. A removal that touched a held entry should be impossible, but if the data ever shows one, treat it as a serious incident: stop the sweeper, keep the evidence, and tell the compliance officers at once. Do not quietly fix it.

Events go out on RabbitMQ so that other parts of the system, such as search indexing, caches and the notebook user interface, can drop their copies. Publishing happens after the commit, never before. A message about a deletion that then rolls back would cause other components to discard data that still exists. If publishing fails, the removal still stands, and the event is retried from an outbox so downstream components catch up eventually. Consumers have to tolerate duplicates and out-of-order delivery, since the retry may deliver an event twice.

No event is sent for a held entry's skip that could leak details of the hold itself. The existence of a legal hold can be sensitive. Audit records are visible to compliance roles, but general scientist-facing notifications should not say "held for legal reasons" unless a policy says so.

## Safety, testing and operations

The test suite for retention-sweeper should be dominated by hold cases, because that is where mistakes cost the most. The cases to keep:

- A candidate past its retention period with `LegalHold` at 1 is skipped, with all its blobs, revisions and derived artifacts intact.
- A hold placed after the candidate list was built but before the delete is honored.
- A hold placed in the middle of a sweep, between two candidates, is honored for the later one.
- A null, missing, or unexpected `LegalHold` value is treated as held.
- A failed read of hold status skips the entry and does not abort other candidates.
- A blob shared between a held entry and an expired one is not removed.
- The delete statement, run directly against a held row with no earlier check, affects nothing.
- An audit write failure prevents the delete.
- A crash between row removal and blob removal leaves a recorded intent that a later pass completes.
- A forced, manual, or on-demand run behaves exactly like a scheduled one with respect to holds.

For change review: any change that touches candidate selection, the delete statement, blob cleanup, or the `LegalHold` read should be reviewed by someone who has read this spec, and ideally by someone from the compliance side. If you are tempted to optimise the hold check away, for example by joining it into the candidate query and trusting it later, do not. The repeated check at delete time is the point.

Dry-run mode is wanted and should be the default for new environments and for any first run after a rules change. In a dry run, the sweeper goes through all of the steps, including the hold checks, and writes what it would have done, but deletes nothing and publishes nothing. Dry-run output is the best way for a compliance officer to see the effect of a rule change before it goes live. It does not replace the hold check in the real run; it only previews it.

Operational notes:

- Run the sweeper off-peak where practical, and keep batches small enough that the SQL Server load stays modest for interactive users.
- Watch for sweeps that finish with many failures. Repeated blob failures mean a storage problem or a permissions change on Azure Blob Storage, and the pending intents will pile up until it is fixed.
- Watch for a sweep that removes nothing for a long time when it used to remove things. That usually means the rules failed to load, or a broad hold covers most of the old data. Either is worth a look, and neither should be "fixed" by loosening the hold check.
- Credentials for the sweeper should be scoped to what it needs. It should not hold rights to alter the hold column. That way a bug in the sweeper cannot clear a hold, and a hold cannot be cleared by the very component that benefits from clearing it.
- Keep the retention rules and the hold data auditable on their own, so that someone can later answer why an entry was removed on a given day.

Open questions to settle with compliance, not to guess at in code:

- Whether a hold applies to an entry only, or can also be set on a project or a person, and if so how the sweeper should read that. Until this is settled, only the entry-level `LegalHold` column is relied on, and anything ambiguous is kept.
- How long a skipped, expired, held entry should keep showing up in sweep summaries after the hold is lifted, and whether lifting a hold should make the entry eligible at once or on the next scheduled sweep.
- What the audit retention rules should say about records of removals, given that those records must outlive the data they describe.

Short version for someone skimming: check `LegalHold` inside the delete transaction, check it again in the delete statement, treat unknown as held, audit everything, delete rows before blobs, publish after commit, and never add a way around the hold.
