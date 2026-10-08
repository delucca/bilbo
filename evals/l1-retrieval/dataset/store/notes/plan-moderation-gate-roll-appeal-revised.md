---
id: 01K88DHT0VDWNBHYJ69V4BXF5N
created: 2025-10-23T08:34-03:00
---

# moderation-gate appeal flow rollout plan

This note replaces the earlier note "moderation gate roll appeal". The new value: roll out the moderation-gate appeal flow to `12 pilot events`, not the earlier plan of 5 pilot events. Everything below assumes the larger pilot. If you find a reference to the smaller pilot anywhere else (a ticket, a channel topic, a checklist), treat it as stale and fix it or point it here.

The appeal flow is the part of moderation-gate that lets a participant contest a hold or a rejection of a question, a poll comment or a chat line, and lets a moderator review that contest and either restore the item or confirm the original call. Until now moderation-gate only decided and recorded. The appeal flow adds a second look, with its own queue, its own states and its own audit trail.

## Why the pilot grew

The smaller pilot was picked because it was cheap to watch. The trouble is that a handful of events does not show us the spread we care about. Event producers run very different rooms. Some are quiet internal all-hands with a friendly crowd, some are public launches with a hostile corner, some are community sessions where the same regulars show up and know each other. Appeals behave differently in each. A pilot too small would likely land on the easy rooms and tell us nothing about the hard ones.

Reasons given for the larger number, as best they were agreed:

- We want enough variety in audience type that at least a few events are clearly adversarial and a few are clearly calm, so the appeal volume per room is not guessed from a single kind of room.
- We want several different event producers and community managers using it, not one team that learns the flow and then hides its rough edges from us by working around them.
- We want a spread of event sizes, including large ones, because the real-time side (WebSockets fan-out of state changes to moderators and participants) is where load shows up first.
- A failed or cancelled event should not gut the pilot. With a small number, one cancellation removes a big share of the evidence. With `12 pilot events`, we can lose a few and still have a usable sample.

The cost is more coordination, more onboarding conversations, more support attention during live hours, and more places where something can go wrong at the same time. The rest of this note is mostly about keeping that cost under control.

## Scope of what ships to the pilot

Keep the pilot scope narrow even though the number of events is larger. The point is to widen the sample, not to widen the feature.

In scope for the pilot:

- A participant can file an appeal against a moderation-gate decision on their own content, from the participant-facing Next.js client, while the event is live and for a short window after it ends.
- The appeal is routed to a review queue that moderators and community managers see in the moderation console, updated live over WebSockets.
- A reviewer can uphold the original decision, reverse it, or send it to a senior reviewer for the event.
- The outcome is shown to the participant who appealed, and when an item is reversed it reappears in the live Q&A or poll stream in a sensible position rather than jumping to the top.
- Every step is written to the audit trail so a producer can read, after the event, who decided what and when.

Out of scope for the pilot:

- Appeals across events, meaning a participant contesting a block that carries over from a previous event.
- Automatic reversal based on any scoring or model output. A person decides every appeal.
- Public display of appeal statistics to attendees.
- Any billing or plan gating of the appeal flow. During the pilot it is simply switched on per event.
- Bulk tools for reviewers beyond the basics already in the console.

If someone asks for an extra capability during the pilot, write it down in the feedback log and do not build it until the pilot review. Changing the flow mid-pilot makes the results across events hard to compare.

## How the flow behaves

Describe it the way a producer would want it explained, because producers are the people who will be asked about it by their own speakers and audiences.

A participant sees a moderation outcome on their item. If the item was held or rejected, the client offers an appeal action with a short free-text reason. The reason is optional but encouraged; we expect reviewers to move faster when it is present. After submitting, the participant sees that the appeal is pending. They are not told who the reviewer is.

On the server, the appeal is an object attached to the original moderation decision. It has a small set of states: pending, under review, upheld, reversed, escalated, and withdrawn. A participant may withdraw their own pending appeal. Only one open appeal per decision is allowed; a second attempt shows the existing one instead of creating a duplicate. Once an appeal is closed it cannot be reopened by the participant, though a senior reviewer can reopen it from the console if new context shows up.

Reviewers see the original item, the reason it was held, the participant's reason for appealing, and the nearby context in the stream so they can tell whether a line that looks harmless alone was part of a pile-on. They should not see more personal data than they already see in the moderation console today. Do not widen what is shown.

When a reviewer claims an appeal, it is marked under review so two people do not do the same work. A claim that sits untouched for too long should expire back to pending. The exact expiry is a tuning value and is deliberately not fixed here; pick it with the pilot producers and record what you chose in the feedback log.

When an appeal is reversed, the item goes back into circulation. For Q&A that means it becomes visible to the room and eligible for the speaker's queue. For polls with free-text input it means the response counts again. For chat-like surfaces it reappears in order of its original time, flagged internally so the audit trail shows it was restored.

When an appeal is upheld, the participant gets a short, neutral message. Do not write copy that sounds like a verdict or a lecture. The tone should read like a calm event staff member talking to an attendee.

## Technical notes

The stack is the usual one: Elixir and Phoenix on the server, Phoenix channels over WebSockets for live updates, CockroachDB for storage, and Next.js for the participant and moderator clients.

Things to keep in mind for the appeal flow specifically:

- State transitions on an appeal must be transactional with the audit write. If the audit entry fails, the transition should not happen. CockroachDB transactions can be retried by the database under contention, so the code that performs a transition must be safe to run again from the top without double-writing or double-notifying. Do the notifications after commit, not inside the transaction body.
- Fan-out of appeal events to reviewers should go to a per-event topic, not a global one. A busy event should not slow down the others. This matters more now that many pilot events may be live at the same time.
- Participant-facing updates should go only to the participant who filed the appeal, over their own session, and never to the whole room. Re-check the channel join authorization for this; a mistake here leaks who is appealing what.
- Reconnects happen constantly at large virtual events. After a reconnect, the client must be able to ask for the current state of its own appeals and, for reviewers, the current queue, instead of relying on having seen every message. Treat the live push as a hint and the fetch as the truth.
- The moderation console in Next.js should degrade gracefully if the socket drops: show that the queue may be stale and offer a manual refresh.
- Per-event switch: the appeal flow is enabled per event through configuration owned by the producer's event settings, so it can be turned off for a single event without a deploy. Make sure that turning it off mid-event leaves pending appeals visible to reviewers and does not strand participants with a button that does nothing. Prefer to hide the action for new appeals and let existing ones finish.
- Idempotency: submitting an appeal twice because of a flaky connection should produce one appeal. Use a client-generated key on the submit request and enforce it on the server.
- Rate limits on submitting appeals per participant should exist so one person cannot flood the queue. Keep the limit generous for the pilot and record how often it triggers.

Nothing here requires a schema redesign of moderation-gate itself. The appeal data is additive. Keep migrations backward compatible so the old code path can keep running if we have to roll back during a live window.

A short sketch of the rollout config, using only the value that matters for this plan:

```
moderation-gate appeal flow
pilot: 12 pilot events
```

## Choosing and onboarding the pilot events

Selection should be deliberate, not first come first served. Build a short list of candidate events from the upcoming schedule and pick so the final set covers the spread described above. Concretely, aim to cover:

- A mix of audience types: internal, public, and community-driven.
- A mix of sizes, with at least some of the largest events we run so the real-time path gets exercised.
- A mix of moderation styles: events with heavy pre-moderation, events with light touch, and events that use a team of volunteer moderators.
- More than one time zone, so support coverage and moderator availability are tested outside the comfortable hours.
- Both Q&A-heavy and poll-heavy formats, since appeals on poll free-text and on Q&A questions feel different.

Avoid putting the flow on an event that is a first-ever event for that producer. They already have enough to learn. Avoid events with a hard legal or compliance sensitivity until the flow has seen some live use, unless the producer asks for it and understands it is a pilot.

Each producer should hear the following before their event, in plain words:

- What an appeal is and who can file one.
- That a person reviews every appeal and that reviewers are their own moderators or community managers, not our staff, unless they arrange otherwise.
- What participants will see and what they will not.
- How to turn it off for their event and who to contact if something looks wrong.
- That we will ask for feedback afterward and that we will read the audit trail for their event to understand how it went, with their knowledge.

Do a short rehearsal with the moderator team before each event when the producer will allow it. The rehearsal is where people discover that they do not know who is supposed to claim appeals, so settle that in advance: name a lead reviewer per event.

## Rollout order and steps

Do not light up all of the events at once. Stage the rollout so problems found early are fixed before they hit the later events.

1. Finish the remaining work on the appeal flow that the pilot depends on: the console queue view, the participant notification, the audit trail entries and the per-event switch. Freeze the pilot scope as described above.
2. Run internal events with the flow on, using our own staff as participants and reviewers, to shake out the obvious breakage. Use scripted bad behavior to generate appeals on purpose.
3. Select the `12 pilot events` using the criteria in the previous section and confirm each with its producer. Keep a short list of spare candidates in case any drop out.
4. Start with the first few events, ideally calm ones, and watch them live with someone from the team in the moderation console alongside the real reviewers.
5. After those first events, hold a quick review. Fix what is broken, adjust copy, tune the claim expiry, and only then continue.
6. Move on to the middle group of events, including the more adversarial rooms and the larger ones. Keep a person on call during each live window.
7. Run the remaining events. By now the flow should be stable enough that live watching can become spot-checking, though the largest event should still get a person watching.
8. Hold the pilot review once the last event has finished and the audit trails are readable. Decide whether to widen the rollout, hold, or change the design.

If a serious problem shows up at any step, stop adding events until it is understood. A serious problem means any of: appeals visible to people who should not see them, an item restored that should have stayed hidden, a participant stuck unable to see the result of their appeal, or the live stream slowing for the room because of appeal traffic.

Do not backfill a dropped event by quietly shrinking the pilot. If events fall out, replace them from the spares so the pilot stays at the agreed size, or say in the pilot review that it ran smaller and why.

## Support, risks and what to watch

Support during live hours is the part most likely to be underestimated. With `12 pilot events` spread over several time zones, someone has to be reachable at odd hours. Agree on a rota before the first event and write down who covers which events. Producers should have a single place to ask for help, and answers should come from someone who has seen the console in use.

Risks worth naming:

- Reviewer overload. If an event draws many appeals and has few moderators, the queue grows and participants wait without news. Mitigation: show reviewers the queue depth clearly, let a lead reviewer escalate, and tell producers to staff appeals the way they staff the rest of moderation. Record cases where the queue got ahead of the team.
- Bad-faith appeals. Some participants will appeal everything, or use the reason field to abuse reviewers. Mitigation: the rate limit, the one-open-appeal rule, and a way for a reviewer to flag an appeal as abusive without escalating the participant automatically. Do not add automatic penalties during the pilot.
- Inconsistent outcomes. Two reviewers may decide similar appeals differently. Mitigation: encourage a short internal note on each decision, and read a sample of decisions from each event during the review. This is a people problem as much as a code problem, and the pilot is where we find out how large it is.
- Reversal confusion. A restored item appearing late can confuse a speaker or the room. Mitigation: place restored items sensibly and let the lead reviewer decide whether a restored question should be raised to the speaker right away or left in the normal order.
- Load. The extra traffic from appeals is small compared with the stream itself, but notification fan-out and reconnect fetches can add up on large events. Watch the Phoenix channels and the database for the largest event closely, and be ready to switch the flow off for that event if it hurts the main experience.
- Data handling. Appeal reasons are free text and may contain personal or sensitive remarks. Follow the same retention and access rules as other participant text, and do not export appeal text outside the event owner without checking what the producer agreed to.
- Rollback. Because the data is additive, switching the flow off per event is safe. Make sure support knows that switching off is a normal, blameless action and not an incident by itself.

What to watch during and after each event, in plain terms rather than as targets:

- How many appeals were filed compared with how many moderation decisions were made, and how that varied with audience type.
- How long appeals waited before a reviewer claimed them, and how long before they closed.
- How often reviewers reversed the original decision. A very high rate suggests the first-pass moderation is too strict; a rate near nothing suggests either the first pass is good or the appeal step is a rubber stamp. Look at examples before drawing a conclusion.
- How often participants withdrew, filed duplicates, or hit the rate limit.
- Whether any appeal was left open when the event closed, and what happened to it.
- Reports from producers and moderators about what confused them.

Keep these as notes per event, in the same shape each time, so the review can compare events side by side.

## Review and decisions after the pilot

The pilot review is a meeting plus a written summary, not a dashboard. Bring the per-event notes, a sample of audit trails, and the feedback from producers and reviewers.

Questions the review needs to answer:

- Did the larger pilot show anything that a smaller one would have missed? Note in particular anything that only appeared in the adversarial or the largest rooms.
- Is the flow understandable to participants without help? Look at the wording of the appeal action and the outcome messages.
- Is the reviewer workflow fast enough for real events, and is the claim and expiry behavior right?
- Are the audit trails good enough for a producer to answer a complaint after the fact?
- Did the real-time path stay healthy at the busiest times?
- Which of the out-of-scope requests came up most often, and which deserve to be next?

Possible outcomes are to widen the rollout to all events with a per-event switch left on by default, to widen it to opted-in producers only, to hold while fixes are made and run a smaller second round, or to rethink the design. Record the decision and the reasons in a new note rather than editing this plan, and link back to this plan from it.

Open items to settle before the first pilot event:

- Who is the named owner for pilot coordination, and who is the backup.
- The support rota across time zones.
- The exact wording of participant-facing copy for pending, upheld and reversed outcomes.
- The claim expiry value and the rate limit value, chosen together with the first producers and written down once chosen.
- Whether producers receive an automatic summary of appeals after their event or only on request.
- Which events are the spares if any of the chosen events drop out.

Until those are settled, do not announce the pilot to producers beyond the ones already contacted.
