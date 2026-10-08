---
id: 01JWBMWB9D8GPRSXKQW3XSD8PY
created: 2025-05-28T11:31-03:00
---

# audit-trail-writer: prev_hash migration to SHA-512

The plan is to move the prev_hash column of audit-trail-writer from SHA-256 to SHA-512, and to have that done by milestone M7. Nothing is migrated yet. This note holds the order of work, the risks and the open questions, so the next session does not have to rebuild them from the code.

The reason is simple. prev_hash is what links each audit record to the one before it. Compliance officers rely on that chain to show that no entry was altered or removed after the fact. A longer digest gives more margin for the retention periods those officers care about, and it lets us say one hash family is used across the whole product. The chain has to stay verifiable the whole way through the change. That is the hard part, not the hash call itself.

## Goal and deadline

- Target: `SHA-512` for every newly written prev_hash value in audit-trail-writer.
- Deadline: milestone `M7`. If it looks like it will slip, say so early. Do not ship a half-converted chain just to hit it.
- Done means: new records are written with `SHA-512`, old records still verify, and the verifier tool passes on a full copy of production-shaped data.

## What is affected

The main place is the audit-trail-writer code that computes the hash of the previous record and stores it. In C# this is probably a single helper, but check for copies. Other places that may touch the value:

- The SQL Server table that holds the audit rows. The prev_hash column is a fixed-width binary or a hex string, depending on how it was declared. A longer digest needs more space, so the column type and any index on it must be checked first.
- Any stored procedure or view that reads or compares prev_hash.
- The RabbitMQ consumers that receive audit events and hand them to the writer. They should not need changes, but confirm that no message carries a precomputed hash with a fixed length.
- Archived audit exports in Azure Blob Storage. These were written with the old digest. They must not be rewritten, because changing them would break the chain they were exported to prove.
- The verifier and any reports used by compliance officers. They must understand both digest lengths.

## Approach

The chain cannot be rewritten in place. A record's hash is computed over the earlier record, so re-hashing history changes every later value and destroys the evidence that the old chain was intact. Instead the chain keeps going and the algorithm changes at a known point.

1. Add a way to tell which algorithm produced a given prev_hash. Either a separate small column recording the algorithm, or a length-based rule. A column is clearer and is the preferred option.
2. Widen the prev_hash storage so it can hold the longer digest. Do this as its own deploy, before any code writes the new value.
3. Teach the verifier to read both. For each record it picks the algorithm from the marker and checks the link.
4. Switch the writer so that new records use `SHA-512`. The first record after the switch hashes the last old record, using the new algorithm over the old record's content, and is marked accordingly.
5. Run both versions of the verifier across the boundary and confirm the chain is unbroken across the switch.
6. Leave old records as they are. Do not backfill.

## Cutover record

Add one explicit audit entry at the point of the switch that says the algorithm changed, when, and from which build. That entry is itself hashed into the chain. A compliance officer reading the trail should be able to find the boundary without reading code.

A sketch of what the writer config could look like. The names are placeholders for whatever the code uses today:

```csharp
// audit-trail-writer: hash selection for prev_hash
var algorithm = HashAlgorithmName.SHA512; // was SHA256 before M7
```

## Risks

- Concurrency at the switch. If two writer instances run during deploy, one old and one new, they may write records with different algorithms in an interleaved order. Either drain to a single writer for the cutover or make sure the marker is written per record so the verifier is never confused.
- Column width. If the old column is too narrow, inserts fail at runtime, and the audit write failing may block the lab workflow that triggered it. Test this on a copy first.
- Rollback. Once any record is written with `SHA-512`, going back means a second boundary, not an undo. Treat the cutover as one-way and write the rollback procedure as another forward switch with its own boundary entry.
- Performance. SHA-512 on 64-bit hardware is usually not slower than SHA-256, but measure with realistic instrument output volume rather than assume.
- Anything outside our code that stores or compares prev_hash, such as a customer-side validation script, will break on the longer value. Find out whether any exist.

## Open questions

- Is the column currently binary or text? This decides the exact schema change.
- Do the archived exports in Azure Blob Storage embed the digest length in their format, and does any reader assume it?
- Who signs off on the change on the compliance side, and do they need a written validation record before `M7`?
- Is a per-record algorithm column acceptable to the schema owners, or do they want a length rule?

## Order of work

1. Inventory every reader and writer of prev_hash, including SQL objects and exports.
2. Decide the marker design and get it agreed.
3. Ship the schema widening on its own.
4. Ship the verifier that understands both.
5. Rehearse the cutover on a restored copy of data and run the verifier across the boundary.
6. Switch the writer, record the boundary entry, and verify again on live data.
7. Write up the validation result for compliance before `M7` closes.

Keep steps 3 and 4 ahead of step 6 in every case. The writer change is small, and it is the only part that cannot be undone.
