---
id: 01KWEXJK69SV9GVXA7XDWAQT3A
created: 2026-07-01T10:23-03:00
---

# entry-sync-service review themes

Notes from going back over the recent reviews of entry-sync-service. Nothing here is a decision. It is what keeps coming up, so the next reviewer does not start cold. The same few themes repeat, and most of them come from the same place: the service sits between scientists who want their entries to just show up and compliance people who want to prove nothing changed.

## Audit trail integrity

The most repeated comment is about the audit trail. Reviewers keep asking whether every state change of an entry leaves a record, including the odd ones: partial syncs, retries, and entries touched by two sources close together. Gaps are rarely in the main path. They show up in error handling, where a branch logs something and moves on without writing the audit record. Reviewers want the audit write and the data write to succeed or fail together, and they say so almost every time.

## Ordering and duplicates

Message delivery through the queue is at-least-once, so the same entry update can arrive twice or out of order. Reviews flag handlers that assume neat order. The usual ask is to make handlers safe to repeat and to say in the code what happens when an older update lands after a newer one. This is mostly about naming the rule, not changing much behavior.

## Instrument output handling

Instrument files are messy. Reviewers point at parsing code that trusts the shape of the output too much, and at places where a malformed file takes down more than that one file. The preference is to quarantine the bad file, keep a record that it was rejected, and carry on with the rest. Vendor differences in format also get called out as a growing source of special cases.

## Blob storage usage

Large raw instrument files go to blob storage, and the entry record points at them. Reviews ask about the gap between the two: what if the blob is written but the record is not, or the other way round. Orphaned blobs and dangling references are the shared worry. Comments also touch on retention and on making sure the stored copy can be tied back to the exact entry version.

## Database access

Comments on SQL Server use are mostly ordinary. Transaction scope is too wide in some places and too narrow in others. A few queries get flagged as likely to slow down as data grows. Reviewers like it when the code is explicit about isolation and about what a retry does to a half-finished write.

## Retry and failure behavior

Retry logic gets attention from every reviewer. They ask whether failures are separated into ones worth retrying and ones that never will succeed, and whether poisoned messages end up somewhere a human can see them. Silent drops are the thing nobody accepts. Backoff is mentioned, but the bigger complaint is missing visibility into what is stuck.

## Time and clocks

Timestamps appear in many comments. Instrument clocks, server clocks and user-entered times do not agree, and the audit story depends on which one is recorded. Reviewers want it clear which time is authoritative for each field, and want time zone handling kept consistent.

## Permissions and attribution

Compliance reviewers care about who did what. Sync actions run as a service, so attribution of the original human author must survive the trip. Reviews note places where the service identity replaces the real author in the record. They also ask that the service have only the access it needs to storage and the database.

## Logging and observability

Logs need to help a compliance officer and an on-call engineer, which are different readers. Reviewers ask for correlation between a queue message, an entry and an audit record, and for less noise in the happy path. They also warn against putting sensitive entry content into logs.

## Tests

Test coverage comes up as thin around failure paths. Happy-path sync is covered well enough. Reviewers want tests for duplicate delivery, partial failure between database and blob, and malformed instrument output. Integration tests that use the real broker behavior are preferred over mocks for ordering questions.

## Code structure

Some classes have grown large, mixing parsing, mapping and persistence. Reviewers suggest splitting them along those lines so the audit rules can be read in one place. Naming is another small repeat: similar words used for different ideas, such as entry, record and revision.

## What to check next time

Start with the audit write on every error branch. Then look at repeat delivery, then the database and blob consistency gap. If a change touches parsing, ask what a bad file does. If it touches identity, ask where the original author goes.
