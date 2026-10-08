---
id: 01KHQWC5Q14H5H62NN5D12RMAW
created: 2026-02-18T05:02-03:00
---

# audit-trail-writer fails on unprovisioned actor (FK_TrailEvent_Actor)

audit-trail-writer fails with Msg 547: The INSERT statement conflicted with the FOREIGN KEY constraint "FK_TrailEvent_Actor" when the acting user has not been provisioned. That is the whole gotcha. The rest of this note is what it looks like in practice, why it happens, how to tell it apart from other failures, and what to do about it. In the team's shorthand the component is called `atwriter`, which is short for audit-trail-writer. Both names show up in logs, chat and tickets, so search for both.

The trap is that the error reads like a database problem, and people lose time in SQL Server when the cause is upstream: someone did something in the notebook or on an instrument, and the person doing it was never set up in the actor store that the audit trail depends on. The write is refused on purpose. The constraint is doing its job.

## Symptom

The service logs a SQL Server error with number Msg 547 and the text saying the INSERT statement conflicted with the FOREIGN KEY constraint "FK_TrailEvent_Actor". The log line usually also names the database and the table the insert was aimed at, which is the trail event table. The column named is the one that points at the actor.

From the user side it shows up differently depending on where the event came from. A scientist may see a notebook entry that saved fine but never shows the expected audit line. A compliance officer may notice a gap: an action they know happened has no trail record. An instrument sync may stall behind the failed message and look like the instrument is slow, when really the writer keeps failing on the same event.

Do not read the absence of an error in the notebook UI as proof that the trail was written. The notebook save and the trail write are decoupled by the queue, so the first can succeed while the second fails later.

## What the constraint actually protects

Every row in the trail event table records who did the thing. That who is a reference to a row in the actor table. The foreign key named `FK_TrailEvent_Actor` makes the database refuse any event whose actor does not exist there. For an audit trail this is intended: an event with no resolvable actor is worth nothing to a compliance officer, because attribution is the point of the trail.

So the constraint should not be dropped, disabled or worked around. Several people have suggested turning it off to unblock a backlog. Do not. A trail with orphaned actors fails the very review it exists for, and re-attaching the actors later is guesswork.

## Root cause

The acting user has not been provisioned. Provisioning means the user has a row in the actor store, created through the normal onboarding path, before they perform any auditable action. When someone acts without that row, audit-trail-writer builds a perfectly valid event, tries the INSERT, and SQL Server rejects it with Msg 547.

Typical ways a user ends up unprovisioned:

- A new hire or visiting researcher who got notebook access through a group or an identity provider rule, but was never put through the provisioning step that creates the actor.
- A service account or instrument account used by an integration, which acts on behalf of the lab but was never registered as an actor.
- A user who was renamed or whose identity changed upstream, so the identifier on the incoming event no longer matches any actor row.
- A restored or cloned environment where the notebook data came across but the actor rows did not, or came across in a different state.
- A user created in one environment and tested in another where they do not exist.

None of these is a bug in audit-trail-writer itself. The writer reports faithfully that the actor is missing.

## How to confirm it is this problem

First check the error text. If it says Msg 547 and names `FK_TrailEvent_Actor`, you are in the right place. A Msg 547 that names a different constraint is a different gotcha; do not apply this note to it.

Second, take the actor identifier from the failing event or the log context and look it up in the actor store. If the row is absent, that confirms it. If the row is present, then the cause is something else, for example a mismatch in how the identifier is formatted or cased between the notebook side and the database, or the lookup running against the wrong database.

Third, check whether the failures are all for one actor or spread across many. One actor points at a single missed provisioning. Many actors at once points at an environment problem such as a restore, a wrong connection target, or a bulk import that skipped the actor step.

## Why the failure is easy to misdiagnose

Several things make this confusing.

The message is a database message, so the first instinct is to look at the schema, the migrations or the SQL Server permissions. The schema is fine and the permissions are fine. The data is what is missing.

The service identity that writes to SQL Server is usually not the person who acted. People mix up the database login with the acting user. The actor in the constraint is the human or account that performed the action, not the account `atwriter` connects with. Granting more rights to the connection login does nothing here.

The failure may be intermittent from the outside. Users who are provisioned write fine, so the system looks mostly healthy, and only the unprovisioned user's events fail. That reads like flakiness when it is entirely deterministic per actor.

## Interaction with RabbitMQ

Events reach audit-trail-writer through RabbitMQ. When the INSERT fails, what happens to the message depends on the consumer's acknowledgement and dead-letter setup. The point to remember is that a message whose actor is missing will keep failing every time until the actor exists. Retrying without fixing the data is wasted effort and can pile up load.

If the failing message is requeued immediately, it can sit at the front and block or slow others on the same queue. If it is routed to a dead-letter queue, it is parked safely but will not be written until someone replays it. Either way, the fix for the data does not replay anything by itself. After provisioning, the parked or retried messages still have to be processed.

Check the dead-letter queue for the trail events when you suspect this problem. A growing count of messages there, all with the same failure reason, is a strong sign.

## Interaction with instrument output

Instrument output is synced into notebook entries, and the trail records that sync. When the actor on an instrument-originated event is an instrument account or a service identity, the same rule applies: it must be provisioned as an actor. New instruments or newly added integrations are a common source of the first failure, because nobody thought of the instrument as a user.

If an instrument starts producing events and the trail shows nothing, while the notebook entries do arrive, look at whether the instrument's identity was registered. The raw data may be sitting safely in Azure Blob Storage while the trail event for it is stuck. That is a good sign for data safety but not for compliance, since the record of who brought the data in is missing.

## Immediate fix

Provision the actor. Use the standard onboarding or provisioning path so the actor row is created the way every other one is, with the right identifier and the right attributes. Do not hand-insert a row with guessed values just to get past the constraint; the attributes on the actor are what the compliance team relies on when they read the trail.

Then get the held events through. If they were dead-lettered, replay them after the actor exists. If they are being retried, they will succeed on the next attempt once the row is in place. Watch the queue drain and confirm the trail rows appear.

Afterwards, tell the user or the lab admin what happened, because they may have noticed missing trail lines and may need to know the gap has been closed and the events were recorded late, not lost.

## Preserving order and timestamps

When delayed events are finally written, they carry the time the action happened, not the time they were inserted. Make sure the replay keeps the original event time. A compliance officer reading the trail should see the action at its true moment, and the insertion time, if the table has a separate field for it, should show it was written late. Do not rewrite event times to hide the delay. The delay itself is information.

Events from the same actor should land in their original order. If you replay by hand, preserve that order. Out-of-order insertion of events for one entry can make a sequence look wrong in review even though every row is individually correct.

## What not to do

- Do not drop or disable `FK_TrailEvent_Actor`, even temporarily, to flush a backlog.
- Do not make the actor column nullable or add a placeholder actor such as an unknown user to absorb failures. That quietly destroys attribution.
- Do not retry in a tight loop hoping it clears. It will not clear until the actor exists.
- Do not delete the failing messages from RabbitMQ to make the alarm stop. Those messages are the only copy of the pending trail events.
- Do not give the writer's database login broader rights as a fix. It is not a permission problem.
- Do not edit the audit rows after the fact to change who the actor was. The trail is supposed to be append-only in spirit.

## Prevention

The durable fix is to make sure no one can act before they are provisioned. A few options, in rough order of how much they help:

Make provisioning a required step of access. Whoever grants notebook access to a person or an account should create the actor in the same step, or the grant should depend on it.

Reject early. If the notebook side checks that the actor exists before accepting an auditable action, the user gets a clear message at the moment of the action instead of a silent trail gap hours later. This moves the failure to where someone can act on it.

Alert on the specific failure. A monitor that fires on Msg 547 mentioning `FK_TrailEvent_Actor` in the audit-trail-writer logs, or on dead-letter growth for the trail queue, turns a silent compliance gap into a ticket the same day.

Cover service identities in onboarding checklists for new instruments and integrations.

## Testing notes

When writing tests around `atwriter`, include a case where the actor is absent and assert that the write is refused, that the failure is surfaced and not swallowed, and that the message is not lost. It is easy to write only happy-path tests with a seeded actor, which is how this class of problem gets past development.

In local and CI databases, seed the actor rows deliberately as part of the fixture, and keep one test that omits them. If a test environment is rebuilt from a copy of notebook data, remember the actor rows have to come along or be recreated, otherwise every test that writes a trail event will trip the same constraint and look like a broken build.

Avoid fixing test failures by loosening the constraint in the test schema. The test schema should match production on this point, or the tests prove nothing.

## Logging and diagnosis hints

The most useful things to have in the log when this happens are the event identifier, the actor identifier as received, the entry or run it relates to, and the SQL Server error number and constraint name. If the log lacks the actor identifier, add it; without it you must dig it out of the queued message body.

Be careful about what ends up in logs. The actor identifier is fine; avoid dumping full event payloads, since they may contain scientific content that belongs to the lab and not to the log store.

When grepping, search for Msg 547, for the constraint name, and for both `atwriter` and audit-trail-writer, since log sources spell the component differently. Scripts and dashboards tend to use the short form.

## Compliance angle

For compliance officers the important questions are whether the gap was detected, how long it lasted, whether anything was lost, and whether the late records are marked as late. This failure mode supports good answers to all of those if handled properly: the database refused the write rather than recording a wrong one, the queue kept the events, and replay restores them.

Record the incident in whatever deviation or issue log the organization uses. State the actor, the window during which events were held, the cause (actor not provisioned), the fix (provisioning and replay), and the check that the counts matched afterwards. If events were genuinely lost, say so plainly. Do not smooth it over.

## Quick triage list

When a report comes in that a trail record is missing:

- Look for Msg 547 and `FK_TrailEvent_Actor` in the audit-trail-writer log.
- Identify the actor on the failing event.
- Check the actor store for that actor.
- If missing, provision through the normal path.
- Check the trail queue and its dead-letter queue for held events and process them.
- Confirm the trail rows exist with the original event times.
- Tell the affected user and, if needed, log the deviation.
- If many actors fail together, suspect a restore, wrong target database or skipped import step, and fix that instead of provisioning one by one.

## Open questions

A few things nobody has settled and that a later session may want to pin down.

Should the notebook refuse an auditable action up front when the actor is unprovisioned, instead of accepting it and failing in the trail later? That is the cleanest prevention, but it changes user-visible behavior and needs agreement from the lab side.

Should audit-trail-writer distinguish this failure from other database failures in its retry policy, so it parks the message at once instead of retrying? Today the behavior depends on general error handling, and a targeted rule for Msg 547 on this constraint would reduce noise.

Is there an automatic path to provision service and instrument identities when they are registered? If so, this class of failure for integrations would mostly disappear.

Until those are decided, the working rule stands: an unprovisioned actor makes audit-trail-writer fail with Msg 547 on `FK_TrailEvent_Actor`, and the fix is to provision the actor and then let the held events through.
