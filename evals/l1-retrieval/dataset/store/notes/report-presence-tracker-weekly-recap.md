---
id: 01KRX3X5483CBWZRSXWMPR72KS
created: 2026-05-18T05:40-03:00
---

# presence-tracker weekly recap

Short recap of the week on presence-tracker. Most of the time went to making the join and leave picture hold up when many people arrive at once, and to cleaning up a few rough edges the moderators noticed during a live rehearsal.

## Where things stand

presence-tracker still reports who is in a session through the Phoenix channel layer, and the Next.js dashboard reads it. It works for normal sessions. The sharp spots are still the busy moments right at the start of an event and right after a network blip.

## Join bursts

The start of a big event is the hardest case. Many viewers connect within a short window and the tracker has to absorb them without stalling the poll and Q&A traffic that shares the same sockets. I looked at how join events are batched before they reach the dashboard. Batching helps, but the flush timing still feels uneven, and I did not settle on a better approach.

## Reconnect handling

Viewers who drop and come back used to show up briefly as two entries. I tightened how a returning socket is matched to its earlier entry. This looks better in manual testing, but it needs a longer run with flaky connections before I trust it.

## Stale entries

Entries for viewers who left without a clean close sometimes lingered. I reviewed the cleanup path and found it depends on a timer that can be starved when a node is busy. I noted it and have not changed it yet.

## Storage side

The durable part of presence data lives in CockroachDB. I checked the write pattern for contention on hot rows during bursts. There is some, and the likely fix is to spread writes or keep more of the state in memory and persist less often. This needs a proper design pass before anyone edits code.

## Moderator view

Moderators wanted the participant count on the dashboard to stop jumping around. I smoothed the display on the Next.js side so it does not redraw on every tiny change. The underlying data is unchanged, so this is cosmetic.

## Tests

I added a few cases around reconnect and duplicate entries. Burst behavior is still only covered by a manual rehearsal, not by an automated test. That gap is the main thing I would like closed.

## Open questions

- How stale can an entry be before moderators are misled?
- Should presence persist at all, or be rebuilt from live sockets after a restart?
- Is the batching window better tuned per event size?

## Next week

Write an automated burst test, then revisit the cleanup timer and the write contention. Hold off on changing the storage approach until the test gives a baseline.
