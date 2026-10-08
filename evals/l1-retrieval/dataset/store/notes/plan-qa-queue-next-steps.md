---
id: 01K88XJGV0N0RPVXPK4KDDHCEV
created: 2025-10-23T13:14-03:00
---

# qa-queue next steps

This is a rough plan for where qa-queue goes next. It is written quickly and stays general on purpose. Nothing here fixes a limit, a threshold or a schedule. Those get settled when each step is picked up. The queue holds audience questions during a live event, lets moderators approve, reject, merge or reorder them, and pushes changes out to everyone watching. The aim now is to make it steadier under load and easier for moderators to live with, without redesigning it.

The related note on the poll side is [[poll-engine-direction-chosen]]. Read it before touching anything that qa-queue shares with polls, because the two should not drift apart on how events are fanned out to clients.

## Where things stand

The queue works for the normal case. Questions come in over the WebSocket channel, get stored in CockroachDB, and show up in the moderator view built in Next.js. Moderators can act on them and the audience sees the result. What is less solid is behavior at the edges: very busy events, reconnecting clients, several moderators acting on the same question, and long events where the backlog grows.

I have not re-measured anything for this note. Any claim about speed or capacity should be checked again before it is relied on. Part of the first step below is getting a baseline we trust.

## Get a baseline first

Before changing the queue, write down how it behaves now. That means a repeatable load scenario that mimics a large event: many viewers connected, a burst of new questions, and a few moderators working at once. Keep the scenario in the repo so anyone can rerun it.

Things to watch in that baseline:

- Delay between a question being submitted and it appearing for moderators.
- Delay between a moderator action and the audience seeing it.
- Database contention on hot rows, especially when many actions land on the same question.
- Memory and process growth in the Elixir side as the backlog grows.
- What happens when a node restarts mid-event.

Record the findings in a separate note, with the actual figures there and not here. This plan only says that the baseline comes first.

## Ordering and concurrency of moderator actions

The biggest correctness risk is two moderators acting on one question at nearly the same moment. Today the last write tends to win, and the loser may not find out. The next work is to make the outcome predictable.

Steps:

- List the actions that can conflict: approve, reject, merge, reorder, mark answered, edit text.
- Decide for each pair what should happen when they collide. Some should be rejected with a clear message, some can be merged quietly.
- Use a version check or equivalent on the stored question so a stale action fails visibly and does not overwrite newer state.
- Make sure CockroachDB transaction retries are handled in one place and do not leak into the channel code as odd errors.
- Show the moderator a plain message when their action lost, and refresh that question in their view.

Reordering needs extra care. A position scheme that rewrites many rows on every move will hurt under load. Look for an approach that touches few rows per move and can be rebalanced occasionally in the background.

## Real-time delivery and reconnects

Clients drop and come back all the time at big events. The queue needs a clean way to catch a client up without resending everything.

- Give each queue change a sequence marker so a reconnecting client can ask for what it missed.
- Fall back to a full snapshot when the gap is too large or the marker is unknown.
- Avoid sending the same update twice to a client that is mid-reconnect.
- Keep the audience view and the moderator view on separate topics so audience traffic stays light and does not carry moderator-only detail.
- Check how updates are batched. Many small pushes in a burst are wasteful; a short batching window may help, but the right window should come from the baseline and not from a guess.

The Phoenix channel and presence code should be reviewed for places where one slow client can hold up others. If any exist, move that work off the shared path.

## Moderation tools

Producers and community managers keep asking for small things that cost little and save time in a live room. None of these are committed. They are candidates to rank once the earlier steps are in.

- Better duplicate detection, so near-identical questions are grouped for the moderator and not shown as separate items.
- Bulk actions on a selection, such as rejecting a run of spam.
- A way to pin or hold a question for the host without approving it for the audience.
- Filters in the moderator view by state, by who handled it, and by whether it has been merged.
- A visible trail of who did what to a question, for review after the event.
- Rate limiting per participant, so one person cannot flood the queue. The shape of the limit is a product question and should be agreed with the people who run events.

Each of these touches both the Elixir side and the Next.js moderator view, so split them into slices that can ship alone.

## Data and retention

Questions pile up across events. Decide what stays hot and what moves out of the way.

- Separate live state, which must be fast, from history, which only needs to be readable.
- Plan how finished events are archived so the live tables stay small.
- Check indexes against the real read patterns of the moderator view and the catch-up path, and drop ones that nothing uses.
- Think about how CockroachDB locality settings interact with where events are mostly run, once there is evidence it matters.
- Confirm what we are allowed to keep about participants and for how long. This is a question for whoever owns policy, not something to settle in code.

## Testing and rollout

Changes to the queue are risky during a live event, so the rollout needs care.

- Add property-style tests for the conflict rules, since hand-picked cases will miss orderings.
- Add a test that kills and restarts a node during a simulated event and checks that no accepted question is lost.
- Use feature flags so new behavior can be turned on for one event and turned off quickly.
- Try each change first on a small internal event before a large one.
- Keep a short checklist for producers on what changed and what to watch for.

## Open questions

- Should the queue and the poll engine share one delivery mechanism, or stay separate? The linked note may already answer part of this; confirm before starting.
- How much history should a moderator see by default?
- Who owns the rules for duplicate merging, product or engineering?
- Is there appetite for a read-only view for co-hosts that cannot change anything?

## Order of work

Baseline first, then conflict handling, then reconnect and catch-up, then data cleanup, and the moderation extras last. If something forces a reorder, the baseline and the conflict rules should still come before anything user-visible. Update this note as steps finish or as the plan changes, and move concrete figures and decisions into their own notes.
