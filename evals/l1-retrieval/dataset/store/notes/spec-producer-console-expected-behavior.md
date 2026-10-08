---
id: 01KZE4H25JWX6F3V1PCQRDSS15
created: 2026-08-07T09:52-03:00
---

# producer-console: expected behavior

This note describes what people expect from producer-console in general terms. It is not a contract and it holds no tuned values. If something here disagrees with what the code does, check which one is wrong before changing either. producer-console is the screen a producer or community manager keeps open during a live event. They run polls, watch the Q&A queue, and moderate while the audience is actively typing. The people using it are under time pressure and often can't look away from the stage to read a tooltip, so most of the expectations below are about being predictable and fast to read.

## Who uses it and what they are doing

There are two main kinds of user. Producers run the show: they decide when a poll opens and closes, which question goes on screen next, and when to show results. Community managers mostly moderate: they approve, reject, merge, and answer questions, and they handle abusive or off-topic content. Sometimes one person does both. Larger events have several people in the console at once, so it has to behave well with concurrent operators.

The console is not the audience view. Attendees never see it, and nothing in it should leak into the attendee experience except through explicit actions such as publishing a result or promoting a question. When we add a feature, the first question is whether it is an operator tool or an audience feature, and it goes in the matching place.

Operators are not necessarily technical. Labels should say what an action does in event terms (close voting, hide question) and not in terms of internal states or table names.

## Live updates

The console is live. It keeps a WebSocket connection to the Phoenix backend and the state on screen follows the server without a manual refresh. Vote counts, incoming questions, moderation changes made by other operators, and poll state changes should all show up on their own.

People expect the following from the live connection:

- New items appear without moving what the operator is already looking at. A question list that jumps under the cursor while someone is about to click is a bug, even if the data is right.
- Counts may update in small batches rather than on every single vote. Operators accept slightly smoothed numbers. They do not accept numbers that go backwards or flicker between values.
- When the connection drops, the console says so clearly and visibly, and it does not keep showing stale data as if it were current. It reconnects on its own and then catches up to the true state, not just to the events it missed.
- After a reconnect the operator should not have to redo anything. Open panels, filters and selections stay where they were.

The server is the source of truth. The console shows what the server says and does not guess at the result of an action and then leave the guess on screen. A small amount of optimistic feedback is fine for responsiveness (a button shows a pending look right away), but the final state always comes from the server and wins over any local guess.

## Polls

Producers create, edit, open, close, and reveal polls from the console. The expected life of a poll is simple: draft, open, closed, and then results shown to the audience or kept private. The console should make the current state obvious at a glance, and the next sensible action should be the most prominent control.

Things people expect:

- A draft can be edited freely. An open poll can be edited only in ways that do not corrupt votes already cast. If an edit would invalidate existing votes, the console says so before applying it, not after.
- Opening and closing take effect for everyone quickly and together. There should be no case where an operator sees a poll as closed while attendees can still vote.
- Closing is not undone by accident. Actions that cannot be reversed, or that are visible to the audience, ask for confirmation in a way that is quick but not automatic. Reversible actions do not ask.
- Results shown to the audience are a separate step from closing. A producer may close voting and look at results privately before deciding to reveal them.
- A producer can see how many people have voted and how the answers are distributed as it happens, with the caveat from above that numbers can be slightly smoothed.

Duplicate votes and vote integrity are handled by the backend, not the console. The console should never offer a way around those rules, and it should show clearly when the backend has rejected something, in the operator's words.

## Q&A queue and moderation

The Q&A queue is where most of the pressure is. Questions come in constantly, and the console is how a human keeps the stream under control before it reaches the stage or the audience.

The expected moderation actions are approve, reject, hide, restore, merge duplicates, mark as answered, and promote to the on-screen slot. Each should work from the keyboard as well as the mouse, because the fast operators use the keyboard. Each should give immediate visible feedback and be consistent in where it sits and what it looks like.

Some general expectations:

- The queue can be filtered and sorted by the things moderators care about: new, approved, answered, flagged, and by how much the audience supports a question. The chosen view persists while new items arrive.
- Bulk actions exist for the bad moments, such as a flood of spam. Bulk actions are clear about how many items they will touch and can be undone when the action is reversible.
- Merging duplicates keeps the support the duplicates collected, and the merged item remembers where it came from, so the moderator can split it back out if they made a mistake.
- Moderation is visible to the team. If one operator rejects a question, the others see that it was rejected and by whom, and they do not see it sit in their queue as pending.
- Automatic filtering (for example, flagging likely abuse) only suggests. A person decides. The console shows why an item was flagged when it can, and a flagged item is never silently dropped.

## Working with several operators

More than one person will act on the same poll or question at nearly the same time. The console should handle that without losing either action and without confusing either person.

When two operators act on the same item, the server decides the order and the loser sees a clear, short notice that something changed, with the new state. It should not just fail or show a generic error. Presence helps: operators can see who else is in the console and, where it makes sense, who is currently looking at or handling an item, so they can avoid stepping on each other. Presence is a hint, not a lock. Nothing should block an operator from acting because someone else has an item open, since a stale lock in the middle of a live event is worse than a collision.

Roles matter. A community manager who can moderate may not be allowed to open a poll or reveal results, and a producer may have wider rights. The console hides or disables what the current user can't do, and says why when it is disabled, instead of letting them click and get refused. The backend enforces the rules regardless of what the console shows.

## Reading the screen under pressure

The layout is built for glancing. The important things are always in the same place: the state of the live poll, the head of the Q&A queue, the connection status, and what is currently on the audience screen. Operators should never have to hunt for any of these.

General expectations on presentation:

- State is shown with words as well as color. A color-only signal fails for some operators and under bad lighting in a control room.
- What the audience currently sees is shown in the console in a way that is clearly separate from drafts and private views. The worst mistake in this tool is thinking something is private when it is public.
- Dense is fine; cluttered is not. Large lists stay usable by keeping rows compact and by virtualizing long lists, so the page stays responsive when the queue is very long.
- Text from attendees is displayed safely and never interpreted as markup. Long or odd text does not break the layout.
- Dark and light modes both work, since control rooms and home offices differ.

The console is built with Next.js, and it should feel like one steady application. Navigating between sections should not drop the live connection or reset the operator's place.

## Data flow

A rough picture of how the pieces connect, kept loose on purpose:

```
producer-console (Next.js)
  <-> WebSockets <-> Phoenix channels (Elixir)
  <-> CockroachDB
```

The console talks to the Phoenix backend over WebSockets for live state and for operator actions. The backend validates each action, writes it to CockroachDB, and broadcasts the result to every connected console and to the audience side. The console does not write to the database directly and has no private channel around the backend.

Because the data store is distributed, an operator's action may occasionally be retried by the backend after a transaction conflict. The console should not be able to cause a double effect from that. Actions are safe to repeat, and if an operator clicks twice, or the connection resends an action after a reconnect, the result is the same as clicking once.

## When things go wrong

Live events do not allow much time to recover, so failure behavior is part of the normal expectations, not an extra.

- If the connection is lost, the console says so right away and stops presenting live data as live. Actions that can't be delivered are shown as not sent. They are never shown as done. If the console queues anything for resend, it says so.
- If the backend is slow, buttons show a pending state, and the operator is not left guessing whether a click registered. A second click does not create a second effect.
- If an action is refused, the message is in plain language, says what happened to the item, and says what the operator can do next. Internal details go to logs and not to the screen.
- If the console itself fails to render a part of the page, the failure stays contained. A broken panel should not take down the poll controls or the moderation queue.
- After any recovery, the console re-syncs from the server so what the operator sees matches reality, and it tells them if anything changed while they were away.

Operators should always have a way to get the most important controls even in a degraded state. Closing a poll and hiding a question are the actions that matter most when something is on fire.

## Things people ask for that we treat carefully

Some requests come up repeatedly, and they need thought before they are added.

A full undo for everything sounds appealing, but many actions are already visible to the audience by the time anyone would undo them. Undo is offered where it truly restores the earlier state and is plainly absent where it cannot.

Auto-approve rules save time but shift responsibility away from a person. If they exist, they are opt-in per event, visible in the console while active, and easy to switch off.

Extra analytics are welcome after the event, but the live console is not the place for heavy reports. Anything that makes the live view slower should live somewhere else.

Per-operator customization of layout is fine as long as the critical items listed above remain in predictable places and a shared screen in a control room does not become unreadable to a colleague.

## Notes for whoever changes this next

Before changing producer-console, think about a producer in the middle of a live event with an audience waiting. Ask whether the change makes any action less obvious, slower, or less reversible, and whether it could cause the operator to believe something is private when it is public.

Keep behavior changes small and test them with several operators connected, with a flaky connection, and with a long queue; most of the real bugs show up in those three situations. When behavior here is deliberately changed, update this note so it keeps describing what people can expect, and leave the exact tuned values to the code and its configuration.
