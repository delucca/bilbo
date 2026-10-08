---
id: 01KXD5TJ7Q8RSHPN5RYXDCCJJ7
created: 2026-07-13T04:24-03:00
---

# pulse-socket weekly recap

Quick recap of the week on pulse-socket. Mostly cleanup and chasing flaky behavior around reconnects and moderation fan-out. Nothing shipped that changes how producers use it, but a few things are better understood than they were.

## Focus this week

Most of the time went into the join and reconnect path, then into how moderation actions reach connected viewers. The Next.js client side got touched only where it had to match.

## Reconnect behavior

Clients that dropped and came back were sometimes treated as brand new joiners. That meant they missed the current poll state until the next update. We traced it to the order in which the channel join and the state replay happen. Replay was racing with the join acknowledgement.

## What changed in the join path

The replay now waits until the join is fully acknowledged on the server. It is a small change in the channel module. It needs more soak time before anyone calls it fixed.

## Poll updates

Vote tallies are pushed to viewers in batches rather than on every vote. This week we looked at whether the batching window feels laggy to producers. Opinions differ. No change made, but the question is open.

## Q&A flow

Question submissions pass through the same socket as votes. Heavy voting can delay a question showing up in the moderator queue. We noted it and did not fix it. Separating the two streams is the likely direction.

## Moderation fan-out

When a moderator hides or removes a question, the removal has to reach every viewer quickly. We found one case where a removal arrived before the question itself on a slow client, leaving a ghost entry. The client now ignores removals for items it has not seen and applies them when the item arrives.

## Database interaction

Writes to CockroachDB from the socket layer were reviewed for retry handling. Transaction retries are expected and handled, but one path logged them as errors, which made dashboards noisy. That logging was downgraded.

## Presence tracking

Presence counts drift slightly after mass disconnects. It corrects itself after a while. We want to know whether the drift is cosmetic or whether it hides leaked entries. Not resolved.

## Load testing

We ran a couple of synthetic sessions against a staging setup. The setup does not look much like a real big event, so treat the outcomes as loose signals only. Memory per connection looked reasonable. Nothing alarming showed up.

## Observability

Added a few more counters around joins, drops and replays. Names are consistent with the rest of the Elixir services. Dashboards still need to be updated to show them.

## Things that bit us

- Local runs hide the reconnect race because latency is too low.
- Test fixtures for moderation events assumed ordering that production does not guarantee.
- Client and server disagreed on how to treat an unknown item id.

## Open questions

- Should votes and questions share a transport, or be split?
- Is the presence drift harmless?
- Is the batching window right for producers who watch tallies live?

## Next week

- Watch the reconnect fix under more realistic conditions.
- Write a test that forces out-of-order moderation events.
- Update the dashboards with the new counters.
- Start a rough design for splitting the vote and question streams.

## Who to ask

The reconnect change was reviewed by one other engineer. Ask them before touching the join path again, since the ordering there is easy to break.
