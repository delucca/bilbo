---
id: 01JR5YAHV0JXNFS5RF8DZ9TDKD
created: 2025-04-06T13:18-03:00
---

# moderation-gate: general direction

We settled on a general direction for moderation-gate: every piece of audience content passes through one gate before anyone else sees it, and the gate sits on the server side of the live path, not in the browser. This note records the direction and the reasons. It deliberately has no thresholds, timings or sizes. Those are tuned separately and change often. If you need a value, look at the config and the code, not here.

The short version: one gate, server-side, decisions stored durably, moderators always able to override, and a failure mode that favors holding content back over letting it through.

## Why a single gate

Before this, moderation checks were spread across the poll path and the Q&A path. Each had its own idea of what "approved" meant. Producers saw questions in the moderator view that the audience view had already shown, or the reverse. Community managers could not explain to a speaker why something appeared.

A single gate means one definition of state for a piece of content, one place to add a rule, and one place to read when something looks wrong. We accepted that this makes moderation-gate a component that many things depend on. We think that is better than several copies that drift apart.

## Where it sits

The gate sits between the WebSocket ingress in Phoenix and everything that fans content out or stores it as visible. Submissions arrive over a channel, go to the gate, and only leave it with an explicit state. Nothing downstream, such as broadcast to attendees, the Next.js audience view, or exports, reads raw submissions. They read gate output.

The Next.js front end is treated as untrusted for this purpose. It can show hints, such as "your question is awaiting review", but it never decides what is visible to others.

```
WebSockets -> moderation-gate -> CockroachDB
```

The block is only the shape of the flow. Broadcast reads from the gate's output side, not from the first hop.

## States and what they mean

Content is always in one of a small set of states: waiting, approved, rejected, or held for a human. We chose to keep the set small on purpose. Extra states tend to appear when someone wants to encode a reason, and reasons belong in a separate field, not in the state.

Transitions are explicit. Content does not move to approved by default or by timeout. An approved item can be pulled back later by a moderator, and that pull-back is a normal transition, not a special case. We want the audit trail to read the same way for automatic and manual decisions.

## Automatic checks versus human review

The direction is layered. Cheap automatic checks run first and handle the clear cases in both directions. Anything unclear goes to a human. We do not want the automatic layer to be the final word on borderline content, because event audiences, and the tone producers want, vary a lot from one event to the next.

Each event can choose how strict the automatic layer is and whether a human must approve everything. The default leans cautious for large public events. Producers can loosen it for small or trusted audiences. This is a per-event setting, not a global one.

The automatic layer should be replaceable. We expect to change what it uses over time, and the gate's contract with the rest of the system should not change when we do.

## Failure behavior

When the gate cannot decide, because a check is slow, a dependency is down or the queue is backed up, content stays held. It is not let through. We chose this because a bad item shown live to a large audience is much worse than a delay in showing a good one.

The cost is that an outage in moderation-gate looks to attendees like their questions vanish into a void. So the front end should show a clear waiting status, and moderators should see a banner when the gate is degraded. Do not hide degradation from the people who can act on it.

Polls are the partial exception: the poll options are authored by producers, not by the audience, so they do not go through the same path. Free text attached to a poll does.

## Moderator experience

Moderators work from a live queue. The direction is that the queue updates in real time over the same WebSocket infrastructure and that actions in it take effect quickly and visibly for everyone. Several moderators can work at once, so two of them can act on the same item. We resolve that with a simple rule: the first recorded decision wins, the second moderator sees the current state, and any later change is a separate, visible action.

We want bulk actions to be possible, since a flood of similar content is the common bad day. Bulk actions still produce one recorded decision per item, so the audit trail stays uniform.

## Storage and durability

Decisions are written to CockroachDB before they take effect for the audience. The database is the source of truth, and any in-memory state in the Elixir processes is a cache that can be rebuilt. If a node restarts mid-event, the gate must come back with the same view of what is approved.

We chose not to rely on process memory alone even though it would be faster. The events are long and public, and losing decisions is not acceptable. Transactions around state changes should be kept small, to avoid contention on busy events where many items change at once.

## Ordering and consistency

Audience-facing order should not depend on the order moderators happen to click. Approval does not reorder content unless the product feature explicitly ranks it, for example by votes. We do not guarantee strict global ordering between different content types. We do guarantee that once an item is rejected or pulled back, no new fan-out of it happens after the decision is recorded.

Clients that were already sent an item before it was pulled back need a retraction message. The gate is responsible for emitting it, and the front end is responsible for honoring it promptly.

## Auditability

Every decision records who or what made it, which rule or moderator, and why where a reason exists. Producers and community managers use this after the event to review incidents, and sometimes during the event to answer a speaker or a complaint.

We keep the reason as free-form plus a small controlled vocabulary. The vocabulary is for reporting. The free text is for humans. Do not try to merge them.

## What we chose not to do

We did not move moderation to the client for responsiveness. We did not make the gate a separate external service for now. It stays in the Phoenix application, with clear module boundaries so it could be split later if load demands it.

We did not add automatic approval on timeout. We did not give the automatic layer power to ban users on its own. Bans and mutes are human actions with a recorded decision.

We also did not try to unify moderation of attendee identity and moderation of content. Those are related but separate concerns.

## Open questions

- How much of the automatic layer's reasoning should moderators see in the queue. More context helps, but it also invites over-trust.
- Whether per-event strictness settings need a preview mode so producers can see the effect before going live.
- How to present degraded-gate status to attendees without causing alarm.
- Whether retraction needs a stronger delivery guarantee for clients that reconnect later.

These are not blockers. They are what to raise if someone proposes changing the direction above.

## How to use this note

If you change moderation-gate, check that your change keeps the following: one gate on the server side, explicit states, held on failure, durable decisions before audience effect, and a full audit trail. If a change breaks one of these, write down why and update this note, so the next session does not have to rediscover the reasoning.
