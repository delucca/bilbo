---
id: 01K1C7QX5659HQ6M4BH8AQ8HTQ
created: 2025-07-29T20:19-03:00
---

# entry-store-schema weekly recap

This week on entry-store-schema was mostly reading and tidying, with a few small changes that touched how entries, revisions and instrument payload references fit together. Nothing here is a finished result. It is a recap for whoever picks the component up next, written fast, so some of it is rough. Where I am unsure I say so instead of smoothing it over.

The component is the relational side of LabNotebook Sync. It holds the notebook entries, the history of every change to them, and the links to the instrument output that arrives through the queue and lands in blob storage. The compliance officers care about the history part more than anything else. The research scientists care that an entry looks the way they left it. Most of the friction this week came from those two wanting slightly different things from the same tables.

The working shape of the data flow, as I keep it in my head:

```text
instrument output -> RabbitMQ -> entry-store-schema (SQL Server) -> Azure Blob Storage
```

The arrow into blob storage is a reference, not a copy of the bytes into the database. That distinction came up several times this week and I come back to it below.

## Where the week went

A large part of the time went into rereading the existing table layout with fresh eyes. The schema grew in layers. The earliest layer was a plain entry table with a body and an author. Later layers added revisions, then signatures, then the instrument attachment links, then the audit rows. You can see the layering in the naming. Older tables use one style for key columns and newer ones use another. It works, but it makes every join a small act of recall. I did not rename anything. Renaming key columns in a store that auditors query directly is a bad idea without a clear migration story, and I did not have one this week.

I spent a good while walking the path of a single entry from creation to a signed state and writing down which tables are touched at each step. The result is a rough list in my own scratch notes, not in the repository. The main finding is that the signing step touches more tables than I expected, and not all inside one transaction boundary in the application code. I checked the code paths that call into the data layer and most of them do wrap the work in a transaction, but I found a spot where a follow-up write for a derived audit row happens after the commit of the main change. I have not proven that this can leave a gap in practice, because the follow-up is retried, but it is the kind of thing a compliance reviewer would flag. I wrote it down as a question and did not change it.

The second large piece of time was on the revision model. Every edit to an entry produces a new revision row, and the entry row itself points at the current revision. The tension is that the entry row is mutable (the pointer moves) while the revisions are supposed to be append only. This works as long as nobody updates a revision row in place. I looked for places in the data access code that update revision rows and found none that do so on purpose. There is one helper that updates a status column on a revision, which is arguably a mutation of history. More on that in the audit section.

Smaller items that got done:

- Read through the indexes on the revision and audit tables and compared them with the queries the reporting side actually runs. A couple of indexes look unused and a couple of obviously useful ones look missing. I only noted these. Dropping or adding indexes on tables of this size is something to do on a copy first.
- Cleaned up some stale comments in the schema scripts that described columns that were since removed.
- Reviewed a pending change to the attachment link table from a teammate and left comments, mostly about nullability and about what happens when a blob is later reported missing.
- Went through the dead letter handling for the queue consumers that write into this schema, to see which failures leave partial rows behind.

## Revisions and the audit trail

The audit trail is the reason this component exists, so most of my attention went here. The model as it stands: every state change on an entry, such as create, edit, sign, countersign, void and attach, results in an audit row that records who, what kind of change, when, and a reference to the revision involved. The audit rows are meant to be written in the same transaction as the change they describe.

Things I confirmed by reading code and schema scripts:

- The audit table has no update path in the application code. There is no stored procedure that updates it either. Good.
- Permissions on the audit table are narrower than on the entry table in the scripts I read. The application login can insert but I did not find a grant for update or delete. I could not verify what is actually deployed in each environment, only what the scripts say. That is worth a check against a live environment by someone with access.
- The timestamp on audit rows comes from the database server clock, not from the application host. That is the right call since application hosts drift, and it keeps ordering consistent within the store.

Things that bother me:

- The status column on revisions that I mentioned. It is used to mark a revision as superseded, signed or voided. A reader who only looks at the revision table sees the current status but not when it changed. The audit table has that information, but the two have to be joined and trusted to agree. If they ever disagree, which one wins? I did not find a documented rule. This is a real gap in the written design, even if the code behaves consistently today.
- The derived audit row written after commit. As above, retry hides the problem most of the time, but if the process dies between the commit and the follow-up write, there is a window. The cleanest fix is to put it in the same transaction. The reason it was not originally is probably that the derived row depends on a value computed by a later step, but I could not confirm that from the history I had available.
- Voided entries. A void is recorded as a change, and the entry stays readable. But the entry list queries filter voided entries out by default, and I found one report query that forgets the filter in one direction and one that applies it in the other. Not a schema fault, but the schema does not make the intent obvious. A computed or indexed flag, or a view that names the intent, would help.

I also thought about whether the audit rows should carry a hash chain, where each row includes a digest of the previous one so that tampering is detectable. This came up in conversation with a compliance officer, who liked the idea in principle. I did not start on it. It touches write concurrency, because a chain forces a serial order on inserts, and with several consumers writing from the queue that could become a hotspot. It needs a proper design discussion first. I am recording that it was raised, not that anything was agreed.

### Signatures

Signature rows reference a revision and a signer and carry a meaning, such as author, reviewer or approver. The schema allows several signatures per revision, which matches the workflow. The constraint I wanted to check was that a signature cannot point at a revision that is not the current one for its entry at signing time. That is enforced in application code, not by the database. A foreign key alone cannot express it. A check through a trigger could, but triggers on this store are not popular with the team, and I agree with the reasons. I left it as application logic and noted that any direct database writes by scripts bypass it.

Related: a few support scripts exist that write directly to the store for data repair. I looked at what they do about audit rows. Some write an audit row and some do not. For a compliance product, a repair that leaves no trace is the worst case. I flagged this to the team and did not touch the scripts. Someone should decide whether repair scripts must always go through the same data layer as the app.

## Instrument output and attachment links

Instrument output arrives as messages, gets parsed by the consumer, and the raw payload is placed in blob storage. The schema stores a link row tying that blob to an entry, along with metadata about the instrument, the run, and the time the data was captured. The link row is what lets a scientist open an entry and see the data next to their notes.

What I looked at this week:

- The link table keeps both a captured time from the instrument and a received time from our side. These can differ a lot when an instrument buffers data or when the network is down. The reporting queries sometimes use one and sometimes the other. I could not tell if that is deliberate. For audit purposes the received time is the trustworthy one, since the instrument clock is outside our control. For scientific meaning the captured time matters. Both should stay, and the docs should say which one a given report uses.
- Integrity information. The link row stores a content digest of the blob so that later reads can verify the payload has not been altered. I confirmed the consumer computes the digest before upload and stores it with the link. I did not confirm that every read path verifies it. The viewer path seems to, and an export path I looked at does not appear to. That is worth a closer look.
- Orphans in both directions. A blob can exist without a link row if the consumer uploads and then fails before the database write. A link row can point at a blob that is gone if someone cleans storage by hand or a lifecycle rule acts on it. The first case is mostly harmless, just wasted storage. The second is serious for an audit trail, because the record says data existed and the data is not there. The schema has a status column on the link table that can say missing, but nothing sets it automatically. A periodic reconciliation job would be the natural answer. None exists as far as I could find.

On the order of operations in the consumer, the sensible order is: upload the blob, then write the link row and audit row in one transaction, then acknowledge the message. If the process dies after upload and before the database write, the message is redelivered and the upload repeats. For that to be safe the blob name must be derived deterministically from the message, so a repeat overwrites the same blob instead of making a second one. I read the naming logic and it appears deterministic, but I would like a test that proves it, since the whole recovery story rests on it.

There is also a question I could not close about large payloads. Some instruments produce big outputs, and the message path may not be the right transport for those. I did not find a schema problem here, but the link table assumes one blob per link. If an instrument emits a set of files for one run, the current practice seems to be several link rows, one per file, with a shared run reference. That works. It would be cleaner with a parent grouping row, but that is a bigger change and not obviously worth it.

## Queue interplay and ordering

RabbitMQ delivers messages at least once, so the consumers have to tolerate duplicates. The schema supports that through a uniqueness constraint on the combination that identifies a message's payload within a run, so a duplicate insert fails cleanly instead of making a second row. I read this part carefully because duplicate handling is where audit trails quietly go wrong: a duplicate that is silently swallowed leaves no trace, and a duplicate that creates a second audit row looks like two events.

What the code does today, as far as I can tell: on a uniqueness failure the consumer treats the message as already processed, logs it, and acknowledges. It does not write an audit row for the duplicate. I think that is right, since the event happened once. But it means the only evidence of the duplicate is in application logs, which have a shorter life than the database. If compliance ever asks how many redeliveries occurred, the database cannot answer. I do not think it should be able to, but it is worth knowing that this is a choice.

Ordering is the other thing. Messages for the same run can arrive out of order, especially after a retry or a requeue from the dead letter path. The schema does not assume order for inserts, but a few queries assume that a later captured time means a later insert. After a replay from the dead letter queue, that stops being true. I looked for queries where this matters and found a couple of display queries that sort by insert order when they probably should sort by captured time. Low risk, but it can confuse a scientist looking at a run that was replayed.

While in there I traced how the consumers handle a database outage. They back off and retry, and messages stay unacknowledged meanwhile. That seems fine. The thing I would watch is a long outage ending in a burst of redeliveries that all hit the store at once, since the audit table is a single write target. I do not have evidence that this has caused trouble. It is a risk I thought of, not one I observed.

## Blob storage references and retention

The store keeps blob references as stored strings plus the digest, rather than any kind of structured address. That keeps the schema independent of how containers are laid out, which I like. The cost is that a change in container layout means a data migration of the references, and nothing in the schema validates that a reference is well formed. A malformed reference would only show up when someone tries to open the data.

Retention came up in a conversation, not in code. Compliance rules for lab records usually require keeping data for a long time, and the people who own storage cost want lifecycle rules that move old data to cheaper tiers or delete it. The schema has no field describing a retention class for an entry or its attachments. If retention is going to be driven by the record, the link table or the entry table needs somewhere to say so. If it is driven by container-level policy, then the schema needs nothing, but the audit trail should record when a lifecycle action moved or removed data, and today it would not know. I did not propose a design. I wrote down the question, and it is on the list for next week.

A related point is soft deletion. Nothing in this store is physically deleted by the application, which is correct for an audit trail. Entries are voided, and links are marked missing or superseded. The risk is the opposite: tables only grow, and with revisions and audit rows being append only, growth is steady. I did not find any archival approach in the scripts. For now this is fine, but a plan for partitioning or archiving the oldest audit rows will be needed eventually, and it must keep them queryable by auditors.

## Migrations, testing and open questions

The migration scripts are applied in order and, as far as I can tell, each is meant to be safe to run once. I read through the recent ones for patterns that might hurt on a large store. Mostly they are fine. Two habits to be careful with: adding a non-nullable column with a default to a big table can lock it for longer than expected on SQL Server, so prefer adding nullable, backfilling in batches, then tightening; and changing a column type on a table that audit queries read can invalidate plans and make the first queries afterwards slow.

I also noticed that the scripts do not consistently record which migration was applied when, beyond the tool's own bookkeeping table. For a regulated product I would like that bookkeeping itself to be treated as part of the audit story: who ran the migration, in which environment, and that the result matched what was reviewed. Right now that lives in the deployment pipeline's history, not in the database. Whether that is enough is a question for the compliance side.

Naming and consistency notes, none acted on: mixed styles for key column names between older and newer tables; some boolean-like columns stored as small integers and some as proper bit columns; a few text columns with no length limit where a limit would be sensible, and a few with a limit that is clearly too tight for pasted instrument notes. Time columns are consistent in being stored in universal time, which I checked on purpose because mixing would be a disaster.

I cannot claim much was verified this week. Most of what is above comes from reading scripts and code, and from talking with teammates. I did not run a load test. I did not inspect a live environment's permissions. I did not replay a dead letter queue against a copy of the store. The tests that exist for this component are mostly integration tests that run against a local SQL Server instance and exercise the happy path for creating, editing and signing an entry. Failure paths are thin. There is no test that kills the consumer between blob upload and database write, and no test that checks duplicate delivery produces exactly one audit row. Those would give me the most confidence for the least effort.

Questions I want answered, roughly in order of how much they worry me:

- Is the derived audit row after commit really outside the transaction, and if so can it move inside?
- Which source wins when the revision status column and the audit rows disagree, and is that written down anywhere?
- Do repair scripts always leave an audit trace? If not, should they be forced through the data layer?
- Does every read path for instrument data verify the stored digest, including exports?
- Who owns reconciliation of link rows against blob storage, and should it exist as a scheduled job?
- How should retention be expressed, on the record or on the container, and how are lifecycle actions audited?
- Is a hash chain over audit rows wanted, and can the write path tolerate the ordering it forces?

Planned for next week, in a loose order: write the two failure path tests and see whether they pass (if they fail, that is the finding); draft a short written statement of the source of truth between revision status and audit rows and get it reviewed by someone from compliance; look at the export path and its handling of the digest; sketch what a reconciliation job would check and report, without building it yet; and go back to the unused and missing index observations and try them on a copy of the store with realistic data.

Things I deliberately left alone, so nobody thinks they were forgotten: renaming of inconsistent columns, any trigger-based enforcement, archival of old audit rows, and the hash chain. Each is a real topic and each needs agreement from more people than me before anyone touches the schema. The component is in reasonable shape for what it does today. The risks I see are about failure windows and unwritten rules, not about the table design being wrong.
