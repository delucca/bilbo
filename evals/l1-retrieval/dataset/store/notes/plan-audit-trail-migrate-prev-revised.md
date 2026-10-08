---
id: 01K7MYJMZ2QQ1A5HB29F391T7Y
created: 2025-10-15T19:06-03:00
---

# audit-trail-writer SHA-512 migration plan

This note replaces the earlier note "audit trail migrate prev". The one change that matters: the migration of audit-trail-writer to SHA-512 now targets milestone M9, which replaces the earlier milestone M7. Anything that still says M7 for this work is out of date.

## Why the target moved

The earlier plan assumed the hash change could ship alone and early. It cannot. The audit-trail-writer sits between the electronic notebook entries, the instrument output that arrives through RabbitMQ, and the stored records in SQL Server and Azure Blob Storage. Changing the digest touches all of those, so it has to land with the work that already touches them. Moving to M9 lets the change ride with that work instead of forcing a second round of validation for compliance officers.

Compliance officers read the audit trail as evidence. A change in how entries are hashed has to be explainable to them without caveats. That is the main reason not to rush it into the older milestone.

## What the migration covers

- The digest that audit-trail-writer computes for each new audit record moves to SHA-512.
- Records already written keep the digest they were written with. We do not rewrite history. Rewriting would break the point of an audit trail.
- Each record needs to say which algorithm produced its digest, so a verifier knows what to recompute. If the current schema has no such marker, adding one is the first piece of work.
- Verification code must accept both the old digest and SHA-512 for the whole transition, and pick the check from the marker on the record.
- Blobs in Azure Blob Storage that carry a stored digest follow the same rule: old ones stay, new ones use SHA-512.
- Messages in flight on RabbitMQ at cutover must still be processed correctly. Consumers have to tolerate either form until the queue has drained.

## Order of work

1. Add the algorithm marker to the audit record schema in SQL Server, nullable, with old rows read as the legacy algorithm.
2. Teach the verification path to read the marker and choose the right digest.
3. Ship that verification change first, so readers can handle SHA-512 before anything writes it.
4. Switch audit-trail-writer to write SHA-512 for new records, behind a setting that can be turned off.
5. Run old and new side by side on a test notebook with real instrument output, and compare verification results.
6. Turn the setting on by default for M9 and tell compliance officers what changed and from which point.

Step 3 before step 4 is the part to not get wrong. If the writer ships first, verifiers in the field will flag valid records as tampered.

## Open points

- Whether column width in SQL Server is already enough for the longer digest. Check before step 1; do not assume.
- Whether any exported report or downstream tool hard-codes the old digest length.
- Who signs off for the compliance side before the default flips.

## Tracking

Keep the target in one place. A sketch of how the plan reads in the tracker:

```text
component: audit-trail-writer
change: SHA-512 migration
milestone: M9
replaces: M7
```

If the milestone moves again, edit this note rather than adding a new one, and say what the previous value was.
