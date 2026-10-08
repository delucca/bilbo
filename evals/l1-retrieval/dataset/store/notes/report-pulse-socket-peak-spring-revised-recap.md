---
id: 01KSJ4H5AMFTR1S1TN5SNW0FGQ
created: 2026-05-26T09:35-03:00
---

# pulse-socket peak load, spring revision

Second pass on how pulse-socket behaved at peak during the spring events, written from memory of what we saw and not from the earlier note. Treat it as partial. Where I say "the usual value" or "the configured limit", look up the real setting before relying on anything here.

## Why this note exists

The spring events pushed pulse-socket harder than anything before. Producers kept big audiences open for a long stretch, and the Q&A and poll traffic arrived in bursts right after a host said "vote now". The point of this note is to keep the shape of the problem so we do not rediscover it.

## What pulse-socket does

It is the Phoenix layer that holds WebSocket connections for attendees, moderators and producers. It fans out poll results, new questions and moderation decisions. Next.js clients connect to it directly after loading the event page.

## Peak shape

Connections ramp up in the ten or so minutes before start, stay flat, then spike messages when a poll opens. The connection count was not the scary part. The message rate right after a poll opened was.

## Connection counts

We ran well above the usual per-node count in the biggest event, but within what the nodes could hold in memory. Memory per connection was lower than feared. Check the dashboards for the real curve; I did not copy the figures.

## Join storms

When a host posted a link in chat, many people joined within seconds. Join handling touched the database for authorization, which made the join path the first thing to slow down. Caching the event membership check helped a lot.

## Poll vote bursts

Votes arrive in a tight window. Writing each vote straight to CockroachDB was fine at normal rates but showed contention on the hot rows holding tallies. We moved toward batching tallies in process and flushing on an interval.

## Result broadcast

Broadcasting every single tally change to every client is wasteful. We throttle result pushes to a fixed interval, using the configured limit, so clients get a smooth update instead of a flood.

## Q&A submissions

Question volume was modest compared with votes, but moderation made it heavier: every question goes to a moderator queue and, once approved, out to everyone. The approve path must not block on the broadcast.

## Moderation latency

Moderators complained when approvals took visibly long under load. The cause was the same mailbox backlog as the broadcast slowdown, not the moderation logic. Separating moderator channels from attendee fan-out fixed most of it.

## Backpressure

Slow clients on bad networks pile up outbound messages. We set a cap on queued messages per connection and drop the connection past the configured limit, so one slow phone does not hurt the node.

## Reconnect behavior

After a brief network blip many clients reconnected at once. The client backoff had too little jitter, so reconnects arrived in waves. Adding jitter on the Next.js side smoothed it out. Server side we also rate limit new joins per node.

## CockroachDB notes

Transaction retries showed up under vote bursts. They were handled, but they added latency. Keeping transactions short and avoiding wide hot keys mattered more than any tuning of the cluster.

## Node scaling

Adding nodes helped linearly for connections but not for the tally rows, which stay a shared hot spot. Load balancing should keep an event's connections spread out, and sticky routing is not needed.

## Observability

Useful signals: mailbox length of the channel processes, outbound queue depth, join rate, and database retry rate. We lacked a good per-event view; that would have saved time during the live event.

## Open questions

Do we want per-event limits that producers can see? Should the vote tally move out of the database path entirely during a live poll? Is the fan-out design still right for the largest audiences, or do we need a tiered broadcast?

## Next steps

Write down the real numbers from the dashboards in one place. Load test the join storm and the vote burst together, since they happen together in practice. Revisit the backpressure cap after that.

## Caveats

This is from a quick recollection. If it disagrees with the earlier note on pulse-socket peak load, trust whichever one matches current config and metrics, and merge the two.
