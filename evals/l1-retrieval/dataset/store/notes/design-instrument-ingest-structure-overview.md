---
id: 01KK84AFBZHC0HRP3C3VRAKSH7
created: 2026-03-08T22:45-03:00
---

# instrument-ingest-worker: overall structure

instrument-ingest-worker is the background service that takes raw output from lab instruments and turns it into something an electronic notebook entry can point at. It is a .NET worker process written in C#. It does not serve users directly. Scientists never talk to it; they see its results as attachments and parsed result tables inside a notebook entry. Compliance officers see it indirectly, through the audit trail it writes. This note is the map of how the pieces fit. It is not a spec and does not fix any limits or tunables. Those live in configuration and in the code, and they change.

The short version: instrument output lands somewhere, a message on RabbitMQ announces it, the worker picks up the message, fetches the file, checks it, stores the original untouched in Azure Blob Storage, parses it into structured records in SQL Server, links those records to the right notebook entry, and writes audit rows about every step. If anything goes wrong it parks the work rather than dropping it.

The design leans on one rule: the original bytes from the instrument are never modified, and nothing is deleted by this component. Everything else (parsed values, links, status) can be recomputed from the originals. That rule shapes most of the structure below.

## Process layout

The worker is a hosted service built on the generic host. Inside it there are a few long-lived background services plus a set of short-lived handlers. I think of it as these parts:

- The consumer layer. It owns the RabbitMQ connection and channel, declares or verifies the queues it needs, and hands each delivery to the pipeline. It does not know what a chromatograph or a plate reader is. It only knows envelopes.
- The pipeline. A sequence of stages that each take a work item and return either a new state of the item or a failure. The stages are listed in the next section.
- The parser registry. A lookup from instrument family and file format to a parser class. Each parser takes a stream and produces a neutral intermediate record set. New instruments are added by writing a parser and registering it, not by touching the pipeline.
- The persistence layer. Two sides: blob access for originals, and SQL access for parsed data, links, work item state and audit rows. Each side sits behind an interface so the pipeline can be tested without Azure or a database.
- The audit writer. A narrow component that all other parts call. Nothing writes audit rows by hand.
- Housekeeping services. Periodic jobs that look for stuck items, reconcile blob contents with database rows, and report health.

The parts are wired with ordinary dependency injection. Handlers are scoped per message, so each delivery gets its own database context and its own audit context. The consumer layer, the registry and the housekeeping jobs are singletons or hosted services. Mixing those lifetimes up was an early source of bugs, so keep the split in mind when adding anything new: if it holds a database context, it must not be a singleton.

There is no in-process shared state that matters for correctness. All durable state is in SQL Server and blob storage, and the worker can be restarted or scaled out without coordination beyond what the broker and the database already give. Several instances can run side by side. They compete for messages from the same queues, and work items carry enough state in the database that a second instance can tell whether a first one already did a step.

## Message flow and pipeline stages

Instruments do not talk to RabbitMQ themselves. Something upstream, an instrument gateway or a file watcher on the instrument PC, drops the file into a landing location and publishes an announcement. The announcement is small: which instrument, which run, where the file is, and enough identity information to later tie it to a notebook entry. The worker treats the announcement as a claim, not a fact. It verifies everything it can once it has the file.

The stages, in order:

- Intake. Read the announcement, validate its shape, and create or find the work item row in SQL Server. The work item is keyed by something derived from the instrument and run, so a redelivered message finds the existing row and does not create a duplicate. This is the idempotency anchor for the whole pipeline.
- Fetch. Pull the file from the landing location as a stream. The worker avoids loading whole files into memory, since some instrument exports are large. Hashing happens while streaming.
- Archive. Write the original to blob storage under a name derived from the content hash and the work item. If a blob with that content already exists, the stage records that and moves on. Only after the blob write is confirmed does the work item move to an archived state. Nothing later runs on a file that is not safely archived.
- Classify. Decide the instrument family and format from the announcement plus a look at the file header. When the two disagree, the item fails classification and is parked. Guessing here would put wrong data in a regulated record.
- Parse. Look up the parser, run it against the archived blob (not the landing copy), and get the intermediate records. Parsers are pure with respect to the outside world: stream in, records and warnings out. They do not touch the database.
- Normalize. Map the intermediate records to the notebook's result schema: units, sample identifiers, timestamps in a single convention, and instrument metadata. Unit handling is conservative. An unknown unit is a failure, not a pass-through.
- Link. Match the sample or run identifiers to notebook entries. This is the stage with the most business rules. A run can match exactly one entry, no entry, or be ambiguous. Exact matches are linked automatically. The others go to a review state where a person resolves it from the notebook side. The worker never picks between ambiguous candidates.
- Publish. Write parsed rows and links in a single database transaction, mark the work item complete, and emit a completion message so the notebook side can refresh the entry. The notification is sent after the commit, from an outbox row, so a crash between commit and publish does not lose it.

Each stage reads the work item state first and skips itself if its outcome is already recorded. That makes replays safe. If a message is redelivered after a crash in the middle of Normalize, the worker walks through the early stages quickly, finds them done, and resumes where it stopped.

Acknowledgement happens at the end of the pipeline run for that message, or when the item has been parked in a durable failure state. The worker does not acknowledge on receipt. That choice trades some redelivery for not losing work, and the idempotent stages are what make it tolerable.

### Queues and routing

There is a main intake queue, a retry path with delayed redelivery, and a dead-letter queue for items that exhausted automatic handling. Completion and review events go out on a separate exchange that other services bind to. The worker does declare what it needs on startup, but it declares in a passive or compatible way and fails fast if the broker topology disagrees with what it expects, rather than silently redeclaring. Topology changes are made deliberately and rolled out with the services that depend on them.

Prefetch is kept modest so that one slow large file does not hold a pile of unacknowledged messages hostage on one instance while others sit idle. Concurrency inside an instance is bounded by configuration, and the heavy stages (fetch, archive, parse) share a bounded pool so memory stays predictable.

## Storage and audit trail

Two stores, with clear roles.

Azure Blob Storage holds originals and a few derived artifacts. Originals go in a container that is write-once in intent: the worker's identity can create blobs and read them but has no rights to overwrite or delete. Immutability policies on the container are the backstop for that, and they are configured on the Azure side, not by the worker. Derived artifacts, such as a normalized export or a rendered preview, live apart from originals so a cleanup of derived data can never reach an original. Blob metadata carries the content hash and the work item reference, so a blob can be traced back even if the database were lost.

SQL Server holds everything relational:

- Work items and their stage state.
- Parsed result rows, with a reference back to the blob they came from and to the parser that produced them.
- Links between results and notebook entries, including who or what made each link and when.
- The outbox for outgoing events.
- Audit rows.

Parsed result rows are treated as reproducible. If a parser is fixed, the old rows are not edited. A new parse run produces a new set, the old set is marked superseded, and both remain visible to audit queries. That is what compliance officers need: a statement of what the system believed at a given time, and why it changed.

### Audit rows

The audit writer records who or what did what, to which record, when, and with what outcome. For this component the actor is usually a service identity plus the worker instance, and sometimes a person, when a review resolution flows back through the worker. Audit entries are appended in the same transaction as the change they describe wherever a database change is involved. For blob operations, which cannot join a database transaction, the audit row is written after the blob is confirmed, and the work item state makes the gap detectable: an archived blob with no matching audit row is something the reconciliation job looks for.

A few properties are deliberate:

- Audit tables are append-only for the application login. No update or delete permission is granted.
- Timestamps come from the database server clock, not from the worker host, so ordering is consistent across instances.
- Audit rows reference stable identifiers, not display names, so a renamed instrument or user does not rewrite history.
- Failures are audited too. A parked item, a rejected file and a manual override each leave a row.
- Audit rows carry a correlation identifier that is also in the logs and in the message headers, so one run can be followed across the broker, the worker and the database.

The schema for audit lives with the shared database project, not inside this component. Changes to it need review from whoever owns compliance requirements, because the shape of those rows is what gets shown to auditors.

## Failure handling, operations and where things live

Failures are sorted into a few classes, and the class decides what happens.

- Transient: broker hiccups, database timeouts, storage throttling. The worker retries in process with backoff for brief problems, then lets the message go to the delayed retry path. These do not change the work item into a failed state.
- Data problems: unreadable file, unknown format, unit not recognized, header mismatch. Retrying will not help. The item moves to a parked state with a reason code and the audit row says so. A person looks at it, and either fixes the upstream cause and requeues, or marks it rejected.
- Ambiguity: link stage cannot choose. This is not a failure in the technical sense. The item sits in review until resolved.
- Poison messages: announcements that cannot even be parsed as envelopes. They go straight to the dead-letter queue with the raw body preserved, and an audit row is written if an instrument and run can be read out of it at all.

Requeueing from the parked state is an operator action that goes through a small administrative path in the worker, which records who requeued and why. Nobody should be editing work item rows by hand in the database. If that happens, the audit trail has a hole, and the reconciliation job will flag it.

### Reconciliation and housekeeping

The housekeeping services do not change data in the normal flow. They look. One compares blobs against work items and reports orphans in either direction. One finds work items that have sat in a non-terminal state longer than they should and nudges them, by republishing the intake message or flagging them for a person. One checks that outbox rows have been delivered and resends the stragglers. These jobs take a database-level lock or lease so that when several instances run, only one does the sweep at a time.

### Observability

Logging is structured, with the correlation identifier, the work item reference and the stage name on every line. Metrics cover queue depth as seen by the consumer, time spent per stage, counts of items by state, and parse outcomes by instrument family. Health checks distinguish liveness from readiness: the process can be alive while the broker connection is down, and in that case it reports not ready so it is not counted as capacity. Sensitive content, meaning actual measurement values and sample names, stays out of logs. Identifiers go in, payloads do not.

### Security and identity

The worker authenticates to Azure and SQL Server with managed or service identities rather than stored keys wherever the hosting allows it. Its rights are the minimum that the stages need: create and read on the original container, read and write on its own tables, append on audit. Broker credentials come from configuration supplied at deploy time. The worker never reaches into the notebook application's database; the only coupling to the notebook side is through the events it publishes and the review states it exposes.

### Extending it

Most changes fall into a few shapes, and each has a natural home:

- A new instrument or file format: add a parser and a registry entry, add classification rules, add sample files to the test fixtures, and add normalization rules if its units or identifiers are new. The pipeline itself should not need edits. If it seems to, stop and think about whether the intermediate record shape is missing something.
- A new linking rule: goes in the link stage and nowhere else. Keep rules ordered and explicit, and make sure every rule that auto-links has a reason string that ends up in the audit row.
- A new stage: only if the work item state model can express it. Adding a state means a migration, a change to the replay logic, and a look at what the reconciliation job assumes. Treat it as a bigger change than it looks.
- Changes to what gets audited: coordinate with the compliance side before changing anything, since removing or reshaping audit output can invalidate prior validation work.

Tests are layered. Parsers have fixture-based unit tests with real, anonymized sample files. Stages have tests against fake persistence interfaces. A smaller set of integration tests runs against a real SQL Server instance and a storage emulator, and checks the replay behavior: kill the handler at each stage boundary, redeliver, and confirm the end state matches an uninterrupted run. That replay test is the one that most often catches mistakes in new stages, so run it whenever the pipeline changes.

### Known soft spots

Things that are awkward and worth remembering when touching this code:

- The gap between blob write and audit row is the weakest point of the audit story. It is covered by reconciliation, not by a transaction.
- Classification relies on both the announcement and the file header, and some instruments write misleading headers. Parsers for those have special cases that are easy to break.
- The link stage depends on identifier conventions that differ between labs. Some of that variation is handled in configuration, some in code, and the line between them is not tidy.
- Large files stress the fetch and archive stages more than parse. If throughput work comes up, look at streaming and bounded concurrency before touching the parsers.
- Superseded parse results accumulate. Nothing prunes them, by design, but queries from the notebook side need to filter on the current set, and a missing filter shows up as duplicate rows in an entry.
