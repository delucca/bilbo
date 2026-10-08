---
id: 01K2STR6002QKF99DQJ2P7XE0Q
created: 2025-08-16T13:17-03:00
---

# qa-queue ranking decay half-life (design)

This note replaces the earlier note about "qa queue ranks questions". The new value: qa-queue now uses a ranking decay half-life of 45 minutes. The earlier half-life was 30 minutes, and that value no longer holds anywhere in the design.

The rest of this note is the design as I understand it now, why the half-life changed, what it touches, and what to check when something looks off. Where I am not sure of a detail I say so instead of guessing. Everything here is deliberately general apart from the half-life itself.

## The change in one place

qa-queue ranks audience questions for a live event. Part of the rank is a time decay: a question's recency weight halves every half-life. That half-life is now 45 minutes. Before, it was 30 minutes.

```text
qa-queue ranking decay half-life: 45 minutes
previous value (retired):         30 minutes
```

A longer half-life means older questions keep their weight for longer. A question posted a while ago stays competitive against fresh ones, so the top of the queue moves more slowly than it did before. Anyone reading the queue as a moderator should notice less churn near the top.

If you see the old value anywhere (a doc, a dashboard annotation, a test fixture, a comment), treat it as stale and fix it or tell someone.

## Why the half-life moved

The short version: with the shorter half-life, good questions sank too quickly in long sessions. In big virtual events a popular question can take a while to collect votes, because most of the audience is not watching the queue all the time. They vote when the host's topic drifts toward it, or when they come back from a break. By then the decay had already pushed it down.

Producers also complained that the queue felt twitchy. Hosts read a question aloud, glance back, and the order had shifted underneath them. A slower decay calms that down without changing how votes count.

The tradeoff is that stale questions linger. A question that was hot early and then became irrelevant stays visible longer. That is handled by moderation (hiding, merging, marking answered) and not by making the decay faster. I would rather keep the ranking function simple and let moderators do the judgment calls.

## What qa-queue is, briefly

qa-queue is the part of TownHall Pulse that holds submitted audience questions for an event, orders them, and serves that order to the people who need it: the host view, the moderator view, and the audience view. It sits next to live polls but has its own rules, since questions are free text, can be duplicated, and need human review before they appear.

The server side is Elixir on Phoenix. Updates reach clients over WebSockets, so a vote or a moderation action shows up without a refresh. Durable state lives in CockroachDB. The web front end is Next.js. The ranking lives on the server, and clients just render the order they are given. That matters for the half-life: changing it is a server-side change, and no client release is needed.

## How ranking works

Each question has a base score driven by audience votes, and a recency weight that decays over time. The visible rank comes from combining the two. The decay is exponential: after one half-life the recency weight is half of what it was, after two it is a quarter, and so on. The half-life of 45 minutes is the single knob for how fast that happens.

I am not recording the exact formula here because it is easy to get wrong from memory. Read the ranking module in the qa-queue code for the real combination. What matters for this note is only that the half-life is a parameter, not a baked-in constant in several places. If you find it baked in somewhere else, that is a bug in the making.

Ranking is computed on the server so every viewer sees a consistent order. Different viewers may see slightly different content (an audience member does not see pending questions), but the relative order of what they all can see should agree.

## Where the value lives

The half-life should be defined once, in the qa-queue configuration, and read from there by the ranking code. Tests should read the same value or set their own explicitly, never assume it. If you are changing the value again, change it in the configuration and then look for anything that duplicated it.

Things worth grepping for after any change: the old value written out in prose, in comments, in test names, in seed data, and in any admin copy that explains ranking to producers. The UI help text is the most likely place for a stale statement, because it is written by hand and nobody runs it.

Do not add a per-event override casually. It was discussed and not done. A single global value is easier to reason about and to explain to producers. If a real need for per-event tuning shows up, write it as its own design note first.

## Effect on live events

For events already running when the value changes, ranks will shift once, because every question's recency weight is recomputed with the new half-life. Older questions will move up relative to where the shorter decay had put them. This is expected and not a bug, but it can look like a jump to a host who is watching closely.

Because the weight is computed from the question's age and not stored as a running total, there is no migration of stored scores. The new value takes effect when the next ranking pass runs. I did not find a reason to backfill anything in CockroachDB for this change.

If you must deploy during a live event, warn the producer first. The queue will reorder slightly and they should not be surprised by it. Better to deploy between sessions.

## Interaction with moderation

Moderation is where the longer half-life shows up in daily use. Moderators approve, hide, merge duplicates, and mark questions answered. None of those actions depend on the decay, but the order moderators see does.

With the slower decay, a pile of approved but unanswered questions stays near the top for longer. Moderators may want to mark things answered more promptly, or hide ones that have gone stale, to keep the visible list useful. Answered questions should drop out of the active ranking altogether and not merely decay, and that rule is unchanged.

Merging duplicates is the case to watch. When two questions merge, the surviving question should carry the votes of both. What it should do with the age is a judgment call I did not change here. Check the merge code before assuming either behavior, since the half-life makes the answer more visible than it used to be.

## Interaction with WebSocket updates

Because updates go out over WebSockets, a ranking change is pushed to connected clients as it happens. A slower decay does not change the message flow by itself, but it does reduce how often pure time-driven reorders are worth sending. Votes and moderation actions still trigger updates as before.

A reconnecting client should get the current order on reconnect and not replay history. That behavior is independent of the half-life, but it is the first thing to check if a client shows an order that disagrees with the server after a network drop.

If time-driven reorders are batched or throttled on the server, the throttle was tuned with the shorter half-life in mind. Worth a look, though I did not find a problem. With a longer half-life there should be fewer meaningful time-only changes, so any throttle that was set generously is probably still fine.

## Interaction with the front end

The Next.js client does not know the half-life. It receives an ordered list and renders it. That is intentional and I want to keep it that way. If a future change makes the client compute rank locally (for optimistic updates, say), the half-life would then have to be shipped to the client, and the two copies could drift. Avoid that unless there is a strong reason.

Optimistic UI for votes can move a question up locally before the server confirms. When the server's order arrives, it wins. With the slower decay the difference between the optimistic and confirmed order should be small, so flicker should be less common than before.

## Testing notes

Tests that depend on time should control the clock rather than wait. Any test that checks that a question's weight halves after one half-life should take the half-life from configuration, so that it survives the next change. Tests that hard-coded the old 30 minutes need to be updated or, better, rewritten to read the configured value.

Useful cases to keep covered: a fresh question with few votes against an older one with many; two questions with equal votes and different ages; an answered question leaving the active ranking; a merge of duplicates; and ordering stability when nothing changes. The last one catches accidental reorders from rounding.

If a test fails after this change, first check whether it assumed the old value. Most failures I would expect are of that kind, not real regressions.

## Monitoring and what to watch

After rollout, watch how often moderators hide or answer questions at the top of the queue, and whether producers report stale questions lingering. If hosts say the queue feels stuck, the half-life may now be too long for short events. If they say it still feels twitchy, the cause is probably elsewhere, such as vote bursts or frequent merges.

Also watch for support messages that quote the old behavior. Producers learn how the queue behaves and may describe it in their own run-of-show documents. A short note to them about the change is worth sending.

I have no hard numbers on the effect yet. This note should be updated when there is real feedback from events run with the new value.

## Open questions

Should the half-life differ for short events and long ones? A fixed 45 minutes is a reasonable fit for long sessions, but a short event may barely see any decay. I have not decided, and I lean toward leaving it alone until someone shows a real problem.

Should merged questions keep the age of the older or the newer one? Today's behavior is whatever the merge code does; verify it before relying on it.

Should answered questions ever come back? Currently they leave the active ranking. A host sometimes wants to revisit one. That is a moderation feature question, not a ranking one.

## Rollback

If the new value turns out to be wrong, rolling back is a configuration change plus a deploy, with no data migration, since scores are not stored with the decay applied. Set the half-life back to the previous value of 30 minutes, or to a new one, and rank will recompute on the next pass.

If you roll back, update this note and the producer-facing help text in the same change, so they do not disagree with the running system. Stale docs about ranking cost more time than the ranking bug itself.

## Summary of what to remember

The half-life is 45 minutes, replacing the earlier 30 minutes. It is a server-side parameter in qa-queue configuration, read in one place. Clients never see it. Changing it reorders live questions once, needs no data migration, and is best deployed between events. Moderation still does the real work of keeping the queue relevant, and the decay only tunes how quickly old questions fade.
