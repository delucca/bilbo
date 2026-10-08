---
id: 01M22E3E6X930GGGD7AJW5XHGR
created: 2026-09-09T03:36-03:00
---

# retention-sweeper orphaned blob investigation

This note records what we found when we looked at why retention-sweeper left so much data behind in the attachments store. Short version: 12% of blobs in the lnb-attachments container were orphaned. They had no matching entry row in SQL Server, so retention-sweeper had nothing to act on and never touched them. The sweeper is not broken in the sense of crashing or skipping rows. It does what it was built to do, and this class of blob falls outside what it was built to see. The rest of this note is the evidence, the working theory on causes, what it means for compliance, and what we would do next. It was written quickly, so treat the open questions section as the honest edge of what we know.

## Naming

reaper9 is the internal codename of retention-sweeper. You will see reaper9 in older chat threads, early design drafts, dashboards that were named before the component got its proper name, and in a few log category strings. In this note and in anything new, use retention-sweeper. If you search logs or tickets for the component, search for both names, because the older material often only says reaper9. They are the same service, deployed the same way, with the same owner. There is no separate reaper9 process and there never was a second implementation behind the codename.

## What retention-sweeper does

retention-sweeper is a background service in the C# and .NET part of LabNotebook Sync. Its job is to apply retention policy to notebook entries and their attachments. It reads entry rows from SQL Server, works out which ones have passed their retention window and are not under any hold, and then removes or archives the associated attachment blobs in Azure Blob Storage. It records what it did into the audit trail so that a compliance officer can later show when and why something was removed. It is driven from the database side: the entry row is the unit of work, and the blob is found by following a reference from the row.

That direction matters for everything below. The sweeper starts from rows and walks to blobs. It never starts from blobs and walks back to rows. A blob with no row is therefore invisible to it.

## How the investigation started

The trigger was a storage cost review. Someone noticed that the size of the lnb-attachments container kept growing faster than the count of live entries would explain, even after retention-sweeper had been running cleanly for a long stretch. Sweeper run summaries looked healthy: rows processed, blobs removed, no errors. Yet the container did not shrink in proportion. Either the sweeper was missing rows it should have caught, or there was data in the container that no row pointed to. We checked the first possibility and ruled it out, then turned to the second.

## Method

We listed the blobs in the lnb-attachments container and compared that listing against the attachment references held in the entry data in SQL Server. A blob counted as matched when at least one entry row referenced it. A blob counted as orphaned when no row did. We did the comparison as a read-only exercise: nothing was deleted, moved, or tagged as part of the investigation. The listing was taken at one point in time, and writes were still happening during the listing, so a small number of in-flight uploads could appear unmatched for a short moment. We handled that by ignoring very recent blobs and by repeating the comparison later to see which unmatched blobs stayed unmatched.

## Result

The result was that 12% of blobs in the lnb-attachments container were orphaned. Those blobs had no matching entry row for retention-sweeper to act on. The share held up when we repeated the comparison after the grace period for in-flight uploads, so it is not an artefact of timing. We measured by blob count. We have not yet produced a clean figure by bytes, and the two could differ a lot, because instrument output files tend to be larger than hand-attached documents. Do not quote the figure as a share of storage cost until someone measures by size.

## Why the sweeper cannot see them

The sweeper selects work by querying entry rows. For each eligible row it resolves the attachment reference and asks storage to remove or archive the blob. If the row is gone, there is no query result that mentions the blob. If the row never existed, the same applies. There is no reconciliation pass in the sweeper that lists the container and looks for blobs without owners. So orphaned blobs accumulate silently. Nothing logs them, nothing fails, and the audit trail has no record of them because no entry ever claimed them.

## Likely causes

We have not proven a cause for each orphan. These are the plausible routes, in rough order of how likely they look from the code paths we know.

### Upload succeeded, row write failed

The attachment path writes the blob first and the row second, across two different systems with no shared transaction. If the row write fails or the process dies between the two steps, the blob stays and the row does not exist. This is the classic dual-write gap and is our leading suspect.

### Message handling and retries

Instrument output arrives through RabbitMQ. When a message is redelivered after a consumer failure or timeout, the consumer may upload the blob again under a new name and then fail on a uniqueness or validation problem when writing the row. The earlier or later copy is left without a row. Redelivery behaviour is worth checking, because it could explain bursts of orphans clustered in time.

### Row removed by another path

Some entry rows may be removed by paths other than retention-sweeper, such as an administrative cleanup, a data correction, or a rollback of a bad import. If those paths delete the row but do not delete the blob, the blob outlives its owner. We have not confirmed that any such path exists in the current code, but it is cheap to check.

### Abandoned drafts

A scientist can start attaching a file to a draft entry and then discard the draft. Depending on how drafts are stored, the blob may remain after the draft row is dropped.

### Older data

Some orphans may predate current behaviour altogether, left by earlier versions of the upload path or by past migrations. We would expect these to be old and stable, whereas orphans from the sources above would still be appearing.

## What we ruled out

We checked whether retention-sweeper was failing to process eligible rows, for example through a stuck cursor, a batch size problem, or an exception swallowed in a loop. It was not. The eligible rows were being handled and their blobs removed. We also checked whether the orphans were blobs that the sweeper had already archived and whose rows had been marked in a way that hid them from the comparison. That was not the explanation either. The orphaned blobs had no trace in the entry data at all, and the audit trail had no removal record for them.

## Compliance implications

This is not only a cost problem. LabNotebook Sync exists to enforce audit trails, and compliance officers rely on the claim that retention policy is applied to everything the system stores. Blobs that no row owns sit outside that claim. They can hold real instrument data or scientific attachments. They might be kept longer than policy allows, or they might be data that was supposed to be kept and has lost its link to the record that gives it meaning. Either way, an auditor asking where a given file came from would not be able to get an answer from the system. We should tell the compliance side about this finding before anyone cleans anything up, because deleting unowned data without a decision about it would itself be an audit event.

## Do not delete yet

It is tempting to write a quick job that removes every unmatched blob. Do not do that as a first step. The matching is only as good as the comparison, and a bug in either side of it would turn a cleanup into data loss. Some unmatched blobs may be the only surviving copy of data whose row was lost through a fault, and a scientist may want it back. Any removal should go through a quarantine step first, with a review window, and the removal should be written into the audit trail like any other retention action.

## Options for fixing it

There are two separate problems: clearing the existing orphans and stopping new ones. They need different work.

### Reconciliation pass

Add a pass that starts from the container instead of from the rows. It lists blobs, checks each against entry rows, and for unmatched ones that are older than a safe age, marks them for review. This could live inside retention-sweeper as a second mode, or in a separate service. Keeping it in retention-sweeper reuses the storage and audit plumbing but widens what the sweeper is responsible for. A separate service keeps the sweeper simple but duplicates access code. We lean towards putting it in retention-sweeper as a clearly separate stage with its own switch, so it can be turned off independently.

### Fix the write ordering

To stop new orphans, change the attachment path so that a row exists before the blob is committed, or so that a blob is only considered live once the row confirms it. One approach is to write a pending row first, upload the blob, then flip the row to confirmed. Pending rows that never confirm can then be found and cleaned by the sweeper, which brings the failure case back inside its field of view. This is the better long-term fix because it removes the gap instead of cleaning up after it.

### Tag blobs at upload

As a lighter measure, stamp each blob at upload with metadata that identifies the owning entry. A later reconciliation can then tell the difference between an orphan and a blob whose row write is slow. It also helps identify who owned an orphan after the fact, which matters for the compliance conversation.

### Lifecycle rules in storage

Azure Blob Storage has lifecycle management that can expire blobs by age. We should not use it as the answer here. It knows nothing about holds or about entry ownership, so it could remove data that policy says must be kept. It might be fine as a backstop for a clearly scratch area, but not for this container.

## Risks to watch

A reconciliation pass reads a very large listing and compares it against a large table. Done carelessly it will be slow and could put load on SQL Server during working hours, when scientists are saving entries. It should run off-peak, work in pages, and tolerate being interrupted. It also has to cope with blobs being added while it runs. A grace period based on blob age is the simplest protection against flagging an upload that is simply not finished.

The second risk is holds. Legal or compliance holds are expressed on rows. An orphan has no row and therefore no hold, which means that if an orphan is actually evidence under hold, the system cannot know. This is another reason to quarantine instead of delete, and to involve compliance in deciding what happens to anything old and unexplained.

## Open questions

- How does the orphan share split by cause? We have theories but no breakdown.
- Is the share growing, shrinking, or flat? A single snapshot cannot tell us. A periodic measurement would.
- What is the share by bytes, as opposed to by count?
- Do other containers or storage areas show the same pattern, or is this specific to lnb-attachments?
- Are there code paths that delete rows without handling blobs?
- What does the audit trail need to say about orphan handling so that an external auditor accepts it?
- Who owns the decision to destroy unowned data, engineering or compliance?

## Suggested next steps

First, put the comparison on a schedule as a read-only report, so we get a trend instead of one number. Second, sample a set of orphans and trace each back through logs and queue history to assign a cause; even a modest sample would show whether the dual-write gap or message redelivery dominates. Third, review all code that removes entry rows and confirm what each does with the blob. Fourth, take the finding to the compliance officers and agree a policy for unowned data before building any deletion. Fifth, design the pending-then-confirmed write ordering and estimate how invasive it is for the upload path and the instrument ingestion consumer. Only after those should we build the reconciliation stage in retention-sweeper.

## Where to look

The sweeper's selection logic and its use of entry rows is the place to confirm the rows-to-blobs direction. The attachment upload handler and the RabbitMQ consumer for instrument output are the places to confirm the write ordering and redelivery behaviour. The audit trail writer shows how removals are recorded today and what a new orphan action would need to add. Search for both retention-sweeper and reaper9 when going through history, since older discussion uses the codename.

## Summary of the finding

retention-sweeper, also known internally as reaper9, works correctly on the rows it can see. The gap is that 12% of blobs in the lnb-attachments container were orphaned, with no matching entry row, so the sweeper had nothing to act on. Fixing that needs a container-side reconciliation stage and a safer write ordering on upload, plus a policy decision with compliance on what to do with unowned data. Until then, do not assume that a clean sweeper run means the container is clean.
