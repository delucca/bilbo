---
id: 01JZQ0V7M78D9YM9QJA9SFAS45
created: 2025-07-09T04:19-03:00
---

# Moderation-gate appeal flow pilot rollout plan

Plan for putting the appeal flow in moderation-gate in front of real events. We roll it out to 5 pilot events during week 42. This note holds the scope, the order of work, the checks we want before and during the pilot, and what would make us stop. Details not settled yet are marked as open, not guessed.

The appeal flow lets a participant whose question or poll response was hidden or rejected by moderation-gate ask for a second look. A moderator on the event then upholds or reverses the decision. Today a rejected item is just gone from the participant's point of view, and producers get complaints in the event chat that they cannot answer quickly. The pilot is meant to show whether the flow cuts that noise without slowing down the moderators.

## Goal and scope

The goal is narrow: prove the appeal flow works under real live traffic on 5 pilot events during week 42, and collect enough evidence to decide on a wider release. It is not a general launch.

In scope for the pilot:

- Participants can file an appeal on an item that moderation-gate rejected or hid.
- Appeals show up in a dedicated queue for moderators, separate from the normal incoming queue.
- A moderator can uphold the original decision or reverse it, and can leave a short reason.
- The participant sees the outcome in the Next.js client without reloading, pushed over the existing WebSocket channel.
- Producers can see appeal counts and outcomes for their own event after it ends.

Out of scope for the pilot:

- Appeals against bans or mutes of a whole account. Only single items for now.
- Automatic reversal based on any score or rule. Every appeal is decided by a person.
- Cross-event appeal history for a participant.
- Any change to how moderation-gate makes its first decision.

## Picking the pilot events

We want 5 pilot events that differ enough to be informative, not copies of the same thing. Aim for a mix of audience size, moderation team size, and event format (mostly Q&A, mostly live polls, and a blend). At least one event should have a large audience and a busy moderation team, because that is where queue pressure shows up first. At least one should have a very small moderation team, because that is where the appeal queue could become a burden.

Selection rules:

- The producer must agree explicitly and know the flow is a pilot.
- The community manager for the event must be reachable during the event, by chat or phone.
- No event with a legal or compliance sensitivity around removed content, until we know how appeals are stored and shown.
- Prefer events whose schedule is known well in advance, so we can do a dry run beforehand.

Open: the final list of events. Someone from the producer side has to confirm which ones fit in week 42. Record the chosen events in this note once agreed, by name, and mark any that drop out.

## Rollout mechanics

The appeal flow ships behind a per-event flag. Nothing is on by default. Enabling it for an event is a deliberate action by us, not by the producer, during the pilot. That keeps the blast radius to the events we picked and gives a one-step way back.

Order of work:

- Finish and merge the backend pieces in the Phoenix app: the appeal record, the transitions (filed, under review, upheld, reversed), and the channel messages.
- Add the CockroachDB migration for appeals ahead of time and deploy it well before week 42. The migration should be additive only, so the old code keeps running against the new schema. Check that it does not take long locks on the busy moderation tables.
- Ship the Next.js participant UI and the moderator queue UI, both hidden when the flag is off.
- Run a full dry run on a staging event with simulated traffic, with a few people playing moderator.
- Turn the flag on for the first pilot event, watch it, then enable the rest as their events come up during week 42.

Do not enable the flag on an event shortly before it starts. Enabling mid-event is allowed only as a recovery step, never as a first start.

## Technical checks before the pilot

Things to verify on the backend:

- Appeals are idempotent: a double click or a reconnect that replays a message must not file the same appeal twice. Decide the dedupe key (participant plus item) and enforce it in the database, not only in the app.
- Concurrent moderators: two moderators opening the same appeal must not both decide it. Use a claim step or a conditional update, and make the loser see a clear message.
- A reversed decision must put the item back where it would have been, including its position or timestamp rules, and must broadcast to everyone who should now see it. Check how this interacts with polls that already closed.
- Rate limits on filing appeals per participant, so one person cannot flood the queue. The exact limit is open and should be chosen with the first dry run.
- Transactions in CockroachDB can be retried. Make sure the appeal handlers are safe to retry and do not send channel messages before the transaction commits.
- WebSocket reconnects: a participant who drops and comes back should still see the current state of their appeal.

Things to verify on the client:

- The appeal button only appears on items that moderation-gate actually rejected or hid, and only for the author.
- The status text is plain and does not reveal moderator identities.
- The moderator queue stays usable with a large backlog and does not freeze on updates.

## What to measure

Decide the metrics now so we do not argue about them afterward. Capture them per event and overall:

- How many appeals were filed, compared with how many items moderation-gate rejected or hid.
- Share of appeals upheld versus reversed. A very high reversal share hints the first decision is too aggressive; a near-zero share hints appeals are a formality.
- Time from filing to decision, as a median and a worst case.
- Effect on the normal moderation queue: did the time to handle ordinary items get worse while appeals were open?
- Abuse signals: repeated appeals on the same kind of content, or many appeals from a few accounts.
- Errors and retries in the appeal handlers, and any dropped or duplicated channel messages.
- Qualitative notes from producers and community managers right after each event.

We do not set numeric pass marks in this note. Agree thresholds with the product owner before the first pilot event and write them here. Until then the criteria are qualitative.

## Support and communication

Producers of the pilot events get a short written brief before their event: what the appeal flow does, what moderators will see, who to contact, and how to ask us to switch it off. Moderators on those events get a quick walkthrough, ideally in the dry run, so the first real appeal is not their first time seeing the screen.

During each pilot event, someone on our side is on call and watching the dashboards and logs for the appeal handlers. Hand-offs between people on call should be written down in the same place so nothing is lost between events in week 42.

After each event, collect feedback within a day while it is fresh. Keep it short: what worked, what confused people, what the moderators would change. Add the findings to this note or to a linked review note.

## Risks and stop conditions

Risks we know about:

- The appeal queue grows faster than a small moderation team can clear it. Mitigation: a per-event cap on open appeals per participant, and a way for producers to pause appeals without turning off moderation.
- Reversals leak content that should stay hidden, for example if a reversal path skips a check that the first path had. Mitigation: route reversals through the same publishing path as normal approvals.
- Database contention on hot rows during a large event. Mitigation: dry run with realistic load, and keep appeal writes off the hottest moderation rows.
- Participants use appeals as a second chance to argue in public. Mitigation: no free-text from the participant that is shown to others; appeals are visible only to moderators.
- Schedule slip: if the backend or the migration is not deployed in time, the pilot moves, we do not squeeze it into week 42 with a rushed release.

Stop conditions during the pilot. Switch the flag off for the affected event at once if any of these happen:

- An appeal causes a rejected item to become visible when the moderator upheld the rejection.
- Duplicate or lost decisions that moderators cannot explain.
- Visible slowdown of the normal moderation flow or the live channel.
- A producer asks us to stop.

After a stop, write down what happened before turning it on anywhere else.

## After the pilot

Once the 5 pilot events are done, review the metrics and feedback together and decide one of three things: widen the rollout, fix and repeat a smaller pilot, or shelve the flow. The decision and its reasons go in a separate decision note about moderation-gate, and this plan gets marked as finished with a pointer to it.

Open items to settle before week 42 starts:

- Final list of pilot events and their contacts.
- Numeric thresholds for the metrics above.
- Rate limit values for filing appeals.
- Who is on call for each event.
- Whether producers see appeal details live or only after the event.
