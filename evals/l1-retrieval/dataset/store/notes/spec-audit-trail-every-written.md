---
id: 01K69SWDBRV3VR9RRYQ2HVG41B
created: 2025-09-29T00:57-03:00
sources:
  - "code: src/AuditTrail/HashChain.cs"
---

# audit-trail-writer spec

This note is the working spec for audit-trail-writer, the component in LabNotebook Sync that records every change to a notebook entry and every instrument import as an append-only audit row. Compliance officers read what it writes. Research scientists never touch it directly, but they feel it when it is slow or when it refuses a write. Keep this note short in spirit: what the component must do, why, and where the traps are. Details that change often (table layouts, queue names, container names) live in the code and are only described in general terms here.

The one rule that everything else hangs on: every row written by audit-trail-writer must carry a SHA-256 digest of the previous row in the column prev_hash. That makes the audit trail a hash chain. If someone edits, deletes or reorders a row after the fact, the chain breaks at that point and a verifier can find it. Most of the design below exists to keep that rule true under concurrency, retries, crashes and restores.

The component is written in C# on .NET. It stores rows in SQL Server, takes work from RabbitMQ, and keeps large instrument payloads in Azure Blob Storage, with only a reference and a content digest in the audit row. If you are changing anything that touches ordering, hashing or transaction boundaries, read the whole note first. Small changes in those places have broken the chain before in ways that only showed up weeks later during a compliance check.

## Purpose and scope

audit-trail-writer exists so that a regulated lab can show who did what to which record, when, and with what result, and can show that the history was not altered. It is not a general event log and not a debugging aid. Anything that does not need to be defensible to an auditor does not belong in it.

What it records: creation, edit, signing, witnessing, reopening and deletion-request of notebook entries; attachment of instrument output to an entry; changes to who may see or edit an entry; and sync events between the notebook and instruments where data was accepted, rejected or altered on the way in. It also records its own administrative events, such as a verification run that found a problem, so that the trail covers itself.

What it does not record: read access for ordinary browsing, keystroke-level editing, transient UI state, and raw instrument telemetry that has not been attached to an entry. Those belong to other components or to ordinary application logs. If a product request asks for something in that list to go into the audit trail, push back and ask whether a compliance officer would actually need it. Every extra row is something the chain has to carry forever.

The component does not decide whether an action is allowed. Authorization happens upstream. audit-trail-writer records what happened, including the identity the upstream service asserted, and it does not second-guess that identity. It does check that the message is well formed, that required fields are present, and that the claimed time is plausible compared with the server clock. A message that fails those checks is rejected and the rejection itself is audited.

Consumers of the trail are the compliance review screens, export jobs for inspectors, and the verifier described below. None of them write. Only audit-trail-writer writes audit rows, and the database permissions are set up so that no other service account can insert, update or delete in the audit tables.

## The prev_hash chain rule

Each audit row stores prev_hash, which is the SHA-256 digest of the previous row in the chain. The digest is computed over a canonical byte form of that previous row, not over whatever the database happens to return. The canonical form is defined once, in a single shared serializer in the codebase, and both the writer and the verifier call it. Do not reimplement it anywhere else. If the writer and verifier ever disagree on the bytes, every row looks tampered.

Canonical form rules, in general terms. Fields are written in a fixed order. Text is encoded as UTF-8 with no byte order mark. Timestamps are written in UTC in a fixed, culture-independent format. Nulls are encoded distinctly from empty strings. Each field is length-prefixed or otherwise delimited so that two different rows can never produce the same bytes by shifting content between fields. The previous row's own prev_hash value is part of what gets hashed, which is what makes it a chain and not just a list of independent digests.

The digest is stored as raw bytes, not as text, in the column prev_hash. Display layers may show it as hex, but the stored value and every comparison use bytes. Compare with a constant-time comparison where the compared value could come from outside the process; for internal verification an ordinary comparison is fine.

What counts as the previous row: the row with the highest sequence position in the same chain at the moment this row is being appended. The sequence position is assigned by the writer inside the same transaction as the insert and is strictly increasing with no reuse. Gaps in the sequence are treated as a defect, not as normal, because a gap can hide a deleted row.

The chain is not one global chain for the whole system. There is one chain per partition, where a partition is a tenant or lab boundary. This keeps contention down and lets one lab be exported or verified without touching another. The rule above applies inside each partition independently.

## First row of a chain

A brand-new chain has no previous row, so the first row needs a defined value for prev_hash. The rule is that the first row of each chain stores a fixed, documented genesis value in prev_hash, derived in the shared serializer from the partition identity and a constant label. It is never null and never an all-zero placeholder. A null in prev_hash anywhere in the table is a bug and the verifier reports it as such.

Why not null: a null makes it easy to insert a fake chain start in the middle of the table and claim an earlier history was archived. A deterministic genesis value that depends on the partition makes a second start detectable, because it would have to be present at a position other than the beginning.

Creating a chain is an explicit, audited operation, not a side effect of the first message. If a message arrives for a partition that has no chain, the writer does not create one on the fly. It rejects the message and raises an operational alert, since in practice this means a configuration problem or a message routed to the wrong partition. Provisioning a new lab goes through the admin path, which writes the genesis row and an administrative audit row saying who requested it.

When archiving old rows to cold storage is eventually done, the archive must keep the boundary row so the live chain still has something to point back to. Archiving is not implemented yet. When it is, the live table must start with a row whose prev_hash still refers to the last archived row, and the archive must be verifiable on its own. Do not truncate the table in the meantime.

## Write path and ordering

The write path for one message goes like this. Receive the message from RabbitMQ. Validate shape and clock plausibility. Resolve the partition. Open a SQL Server transaction. Lock the chain head for that partition. Read the head row. Compute the SHA-256 digest of the canonical form of that head row. Build the new row with that digest in prev_hash and the next sequence position. Insert it. Update the chain head record to point at the new row. Commit. Only then acknowledge the message to RabbitMQ.

Ordering is the hard part. Two writers appending to the same partition at once would both read the same head and both produce a row claiming the same previous row, which forks the chain. The fix is that appends for a partition are serialized by the head lock taken inside the transaction. The lock is held for the shortest time possible: hash computation and insert only. Anything slow, such as blob uploads or lookups, happens before the transaction is opened.

Throughput comes from running many partitions in parallel, not from running many appenders on one partition. Within a partition, throughput is bounded by commit latency. If a single lab produces more volume than that allows, the right answer is batching several messages into one transaction, appending them in order with each row hashing the one before it, and committing once. Batching is allowed as long as the order inside the batch is the order of the messages as consumed from the queue for that partition.

The clock is never used for ordering. Each row carries an event time as claimed by the sender and a recorded time as set by the writer, but the chain order is the sequence position. Reviewers sometimes ask why event times can look out of order in the trail. That is expected: the trail shows the order in which things were recorded, and event time is just data.

The writer must not retry an insert in a way that can produce a duplicate row for the same message. Each message carries a stable identity assigned upstream, and the writer stores it with the row under a uniqueness rule per partition. A redelivered message that has already been written is detected by that rule, treated as success, and acknowledged without writing again.

## Transactions and SQL Server details

The database is SQL Server. The audit tables are append-only by permission: the writer's account has insert and the minimum read rights it needs, and nothing that allows update or delete on audit rows. The chain head record is the single mutable thing and lives in a separate small table that only the writer touches. Do not give the writer's account broader rights to make a migration easier; run migrations under a separate deployment identity.

Isolation. The head lock is taken explicitly, not left to the isolation level. Use an update-style lock on the head record for the partition so that concurrent writers queue up instead of deadlocking when they both try to upgrade. Reading the head row after the lock is held is then safe at the default read-committed level. Do not switch the writer to a snapshot-style level to reduce blocking, because a stale head read is exactly the fork we are trying to prevent.

Transaction scope. One transaction covers: head lock, head read, insert, head update. Nothing else. Do not enlist RabbitMQ or Blob Storage in the transaction. They cannot take part in it anyway, and the design handles the mismatch with ordering and idempotency instead (see the intake and blob sections).

Timeouts. The lock wait must have a bounded timeout. When it expires, the writer rolls back, does not acknowledge the message, and lets it be redelivered. A burst of lock timeouts for one partition usually means a long-running transaction elsewhere or a stuck verifier holding a heavy read; look there first before raising the timeout.

Schema changes. Adding a column to the audit row changes the canonical form, and therefore changes what future rows hash. Old rows must still verify with the old form. So the canonical serializer is versioned, each row records which version it was written under, and the verifier picks the matching version per row. Never change the meaning of an existing version. Add a new one. Never edit past rows to "upgrade" them, since that would break the chain by definition.

Backups and restores. A restore to an earlier point silently drops recent rows, and the chain still looks valid because the latest remaining row is internally consistent. Therefore the head position and head digest are also published periodically to a separate location outside the database, so that a rollback shows up as a mismatch. A restore must be treated as an audited incident, not a routine operation.

## RabbitMQ intake

Audit messages arrive through RabbitMQ. Producers are the notebook service, the instrument sync service and the admin tooling. Each producer publishes to an exchange that routes by partition, so that all messages for one partition land on the same queue, and one consumer at a time processes that queue. That single-consumer-per-partition arrangement is what keeps ordering meaningful before the head lock even comes into play.

Acknowledgement is manual and happens after commit. If the process dies between commit and acknowledgement, the message is redelivered and the uniqueness rule on message identity turns the second attempt into a no-op. That is the intended at-least-once plus idempotent-write combination. Do not change to automatic acknowledgement for speed. It would lose audit rows on a crash, which is the worst failure this component can have.

Prefetch is kept small. A large prefetch window means many unacknowledged messages held by one consumer, which delays redelivery to others after a failure and makes memory use hard to predict. Batching in the writer is done by reading what is already available up to a configured limit, not by raising prefetch without bound.

Poison messages. A message that fails validation is not retried forever. It is routed to a dead-letter queue after it is rejected, and the writer appends a rejection row describing what failed and which producer sent it, without copying the whole offending payload into the chain. Operators review the dead-letter queue. Replaying from it goes through the normal intake path and the normal validation, never through a back door insert.

Ordering across redelivery. If a message is rejected back to the queue for a transient failure, it can come back after later messages in the same queue. For audit purposes that is acceptable only if the producer's own sequence is carried in the message and recorded in the row, so the reviewer can see the producer's intended order beside the chain's recorded order. Producers that do not send such a sequence are a known gap and are listed in the open questions.

Connection handling. The consumer reconnects with backoff on connection loss and does not process anything while it is unsure of its channel state. A consumer that loses its channel mid-transaction must roll back, because the acknowledgement it would send no longer means anything.

## Blob storage for instrument payloads

Instrument output can be large: raw files, images, spectra, run logs. Those do not go into SQL Server. They are stored in Azure Blob Storage, and the audit row holds a reference to the blob and a content digest of its bytes. The content digest is also SHA-256, but it is a different thing from prev_hash and is stored in its own field. Mixing them up in code or in conversation has caused confusion before: prev_hash is about the previous row, the content digest is about the payload.

Order of operations. Upload the blob first, verify the stored size and digest against what the sender claimed, and only then open the database transaction and append the row. If the upload fails, no row is written and the message is not acknowledged. If the upload succeeds and the row insert later fails, an orphan blob remains. Orphans are harmless to the trail and are cleaned up by a separate sweep that removes blobs not referenced by any audit row after a safe delay. The reverse case, a row pointing at a blob that does not exist, must never happen, and the ordering above is what prevents it.

Immutability. The container used for audit payloads has a retention policy that forbids overwrite and delete during the required retention period. This is configured on the storage side and checked at startup; the writer refuses to start if the policy is missing, because the digest in the row is only meaningful if the bytes cannot change underneath it.

Blob naming is derived from the partition and the content digest, so uploading the same bytes twice lands on the same name and is naturally idempotent. Do not put user-entered text such as sample names or entry titles into blob names. They can contain characters that cause trouble and they can be sensitive.

Access. The writer has write-once rights to the container. Readers use short-lived, read-only access issued by the review application after its own authorization check. Nobody gets a standing key. If an inspector export needs payloads, the export job copies them out with its own identity and records that fact in the trail.

When the blob content digest on read does not match the one in the row, that is a tampering or corruption finding and is handled as an incident, the same as a chain break.

## Verification

A chain is only worth something if it is checked. There is a verifier, separate from the writer, that walks a partition from the start or from a trusted checkpoint, recomputes the SHA-256 digest of each row in canonical form, and compares it with the prev_hash stored in the next row. It also checks sequence positions for gaps or repeats, checks that no prev_hash is null, checks that the genesis value is where it should be and nowhere else, and checks that the head record agrees with the last row.

The verifier uses the same canonical serializer as the writer, selected by the version recorded on each row. It runs read-only, with its own database identity that cannot write to the audit tables. It must not take the writer's head lock, since that would stall appends. Instead it reads at a consistent point and verifies up to the position it saw at the start, ignoring rows appended after.

Schedules. A light check of recent rows runs frequently, mostly to catch problems quickly. A full walk of each partition runs less often and on demand before inspections. Checkpoints, meaning a known good position and digest recorded by a verification run, let later runs start from there instead of from the beginning. A checkpoint is itself audited and also published to the external location mentioned in the database section, so that rewriting history inside the database cannot also rewrite the checkpoints.

What the verifier reports: the partition, the first position at which a mismatch was found, what kind of mismatch it was (digest mismatch, gap, repeat, null, version unknown, head mismatch, blob digest mismatch), and the rows around it. It does not try to repair anything. Repair would mean editing audit rows, which is exactly what must be impossible. The response to a finding is operational: freeze the partition's writes if needed, investigate, and document.

Tests for the verifier include deliberately corrupted copies of a chain: a changed field in a middle row, a deleted row, two rows swapped, a row inserted with a plausible but wrong prev_hash, and a truncated tail. Each must be detected and reported at the right position. Keep these tests; they are the closest thing to a specification of what "tamper-evident" means here.

## Failure handling

Principle: when in doubt, do not write and do not acknowledge. A message left on the queue can be handled later; a wrong row in the chain cannot be removed. This pushes the failure cost onto availability, which is the correct trade for an audit trail. Producers must tolerate delayed audit writes and should buffer or retry on their side.

Head inconsistency. If the writer reads the head and finds that the head record disagrees with the last row, or that the stored digest chain does not hold locally around the head, it stops writing for that partition, raises an alert, and keeps consuming nothing from that queue. It does not try to guess which side is right. A human decides, and what they decide gets recorded as an administrative event after the partition is unfrozen.

Transient database errors. Deadlock victim, timeout, brief connectivity loss: roll back, do not acknowledge, let redelivery handle it, with backoff to avoid a tight loop. Permanent errors such as a constraint violation other than the message identity rule are treated as bugs: log with enough detail, stop the partition, alert.

Blob errors. Upload failure leaves the message unacknowledged. A digest mismatch between the claimed and the stored bytes causes rejection with a rejection row, since the sender gave us something inconsistent.

Crash safety. Because acknowledgement comes after commit and writes are idempotent per message identity, a crash at any point leaves the system either with the row written and the message redelivered once more (harmless) or with nothing written and the message redelivered (normal). There is no point in the sequence where a message is acknowledged without its row being committed.

Clock problems. If the server clock jumps, the plausibility check on claimed time could reject valid messages or accept bad ones. The writer logs a loud warning when the recorded time of a new row is earlier than the previous row's recorded time, and does not reorder anything because of it. Time sync on the hosts is an operations responsibility.

Shutdown. On a graceful stop, the consumer stops taking new messages, finishes the in-flight transaction, acknowledges, and exits. On a forced stop, the crash-safety argument above applies.

## What not to do

This list comes from near misses and from review comments. Treat each item as a real trap.

Do not compute prev_hash from the row as read back through an ORM entity that has been normalized. Time zones, trailing whitespace and decimal scale can change on the way, and the bytes will differ from what the verifier computes. Always hash the canonical form built by the shared serializer from the stored values.

Do not hash the new row's own content into its own prev_hash. The field holds the digest of the previous row, full stop. A row's own content shows up in the next row's prev_hash.

Do not read the head outside the lock and then take the lock afterward. The read must be inside the locked section, or two writers will fork the chain.

Do not add a convenience update path, such as fixing a typo in an audit row, even for admins. If a correction is needed, append a new row that states the correction and refers to the earlier row. The earlier row stays as it is.

Do not swallow exceptions around the insert and carry on to acknowledge. Several earlier drafts did this to keep the queue moving, and it would lose rows without anyone noticing.

Do not reuse sequence positions after a rollback by reading the maximum later and assuming it is contiguous. Positions are assigned inside the locked transaction from the head record, so a rolled-back attempt leaves no trace and the next attempt gets the same position legitimately. Anything assigning positions outside that transaction, such as a database sequence object, would leave gaps on rollback and make gap detection useless.

Do not put personal data of research subjects into audit rows if a reference would do. The trail is kept for a long time and cannot be edited, so it cannot honor later erasure requests. Store identifiers and let the source system own the personal data.

Do not change the canonical form, the hash algorithm or the field order without a new serializer version and a written migration note. Switching the algorithm from SHA-256 to something else would need the same treatment, with old rows continuing to verify under the old version.

Do not run load tests against a real partition. Use a throwaway partition and discard the whole thing after, since test rows cannot be deleted from a real chain.

## Testing notes

Unit tests cover the canonical serializer heavily, because it is the single point whose behavior must never drift. Known-answer tests pin the bytes and the resulting SHA-256 digest for a handful of representative rows, including rows with nulls, empty strings, non-ASCII text, and time values at boundaries. If one of these tests has to change, that is a format change and needs a new serializer version, not an edited expectation.

Property-style tests generate random sequences of rows, append them through the writer logic against a test database, and then run the verifier, which must report a clean chain. A second pass mutates one random thing and the verifier must find it at the right position.

Concurrency tests start several writers against the same partition and a large number of messages, then check that the chain has no forks, no gaps and no repeats, and that every message identity appears exactly once. These tests are slow and are run in the nightly pipeline, not on every commit, but they must be run before any change to locking, isolation or transaction scope is merged.

Fault injection tests kill the process at each step of the write path: before the transaction, after the insert but before commit, after commit but before acknowledgement. They then restart and confirm the end state matches the crash-safety reasoning above. A similar set drops the RabbitMQ connection and makes the blob upload fail partway.

Integration tests use a real SQL Server instance in a container, a real RabbitMQ in a container and a storage emulator for Blob Storage. Mocking the database for the write path has been tried and it hid exactly the lock and isolation bugs that matter, so do not do it for anything beyond pure logic tests.

Before a release, a verifier run over a copy of a realistic partition must come back clean, and the result is attached to the release record.

## Open questions

Producer sequence. Not every producer sends its own sequence number with messages. The instrument sync service does; some older notebook paths do not. Until they do, the recorded order is the only order we have for those messages, and a reviewer cannot tell whether redelivery reordered anything. Needs a decision with the notebook team on whether to require the field and reject messages without it.

External publication of head digests. The idea of publishing the head position and digest outside the database is accepted, but the target is not settled: a separate storage account with immutability, a signed timestamp from a trusted service, or both. Compliance wants something an outside party can check. Engineering wants something cheap to operate. Unresolved.

Archiving. Described above only as a constraint. The design for moving old rows out while keeping the live chain verifiable, and for verifying the archive independently, has not been written. It will need its own note.

Partition granularity. One chain per lab boundary works now. Some labs are large enough that commit latency on a single chain may become the limit. Splitting a lab into several chains would complicate cross-record questions such as "show me everything that happened to this entry" if entry activity spans chains. Probably keep entries within one chain and split by entry group, but this has not been tested.

Signing of checkpoints. Verifier checkpoints are currently trusted because they are stored and published as described. Signing them with a key held outside the application would raise the bar further. Key custody and rotation are the sticking points, and whoever owns that should be named before work starts.

Retention end of life. When the retention period for a partition ends and data may legally be removed, the process for removing it without breaking verification of the remainder is undefined. Likely answer: remove only whole partitions or whole archived segments with an audited, documented act, never individual rows. Confirm with compliance.
