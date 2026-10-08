---
id: 01KJS1NSMDM37EYNM72V0W5P1F
created: 2026-03-03T02:10-03:00
---

# pulse-socket peak load at the autumn summit

This note replaces the earlier note "pulse socket peak spring". The new peak value is 260000 concurrent connections, held across 18 nodes at the autumn summit. The earlier peak was 210000 concurrent connections.

In this note `psock` is short for `pulse-socket`. Both names show up in dashboards, chat and runbooks, and they mean the same component. It is the Phoenix and WebSockets layer of TownHall Pulse. It carries live poll votes, Q&A traffic and moderation actions between the Next.js front end and the rest of the Elixir backend.

## What was measured

At the autumn summit, `pulse-socket` held 260000 concurrent connections across 18 nodes. That is the highest concurrency we have seen on this component in production. It replaces the spring figure, which was 210000 concurrent connections.

Treat the number as an observed peak, not a tested ceiling. It is what the fleet carried on the day with the node count we had running. We did not push until something broke. Nobody should quote it as the maximum the system can do.

The figure counts open WebSocket connections held at the same moment, not unique attendees over the whole event. Some attendees reconnect, and some have more than one tab open, so the two numbers differ. When someone asks "how many people were there", this note does not answer that. It answers how many sockets were open at once.

## Why the old note is superseded

The spring note gave a smaller peak and was written for a smaller event. Anyone planning capacity from it would under-provision. Keep the spring figure only as history: it shows how fast peak concurrency grew from one big event to the next.

If a later event beats this value, write a new note or update this one. Do not leave two competing peak values around. Searching for the peak should land on one note that holds the current number.

## Naming

Use `pulse-socket` in anything written for other people. Use `psock` only where the short form is already established, such as the conversation shorthand in the team channel. Do not invent a third name. Older material may say "the socket tier" or "the gateway", and those refer to the same thing. If you see them, read them as `pulse-socket`.

## What the peak means for capacity planning

The useful way to plan is per node. Divide the peak by the node count to get the average load each node carried at the summit. Work that out from the two numbers in this note rather than copying a derived figure from someone's slide. The average hides skew, because some nodes always carry more than others, so leave headroom above it.

For the next big event, start from the autumn numbers and add margin. The main reasons to expect more load are larger audiences, longer sessions with fewer drop-offs, and more interactive formats such as back-to-back polls. A single poll that opens for everyone at once creates a burst of votes. That burst matters more than the steady connection count.

A few things to check before an event of similar or larger size:

- Node count is set ahead of time, not left to react after the audience arrives.
- Connection limits at the load balancer and the operating system match what each node is meant to hold.
- Memory per connection has been rechecked since the last release, since a small per-connection change multiplies at this scale.
- The database behind it, CockroachDB, has headroom for the write burst from votes and questions.

## Where the pressure showed up

This section is general on purpose. We have no measured breakdown in this note, and it does not claim one.

The pressure points in a system like this are well known. Fan-out is the first: one moderation action or one poll result has to reach every connected client, so broadcast cost grows with the number of sockets. Reconnect storms are the second: if a node drops, its clients come back together and hit the remaining nodes. The third is write contention in the database during vote spikes.

When reviewing the autumn run, look at those three areas first. If a later note records actual measurements, link it here instead of repeating the details.

## Moderation and real-time behaviour

Real-time moderation is a core promise of the product. Producers and community managers hide, pin or remove items while the event is live, and the audience expects the change to show up quickly. At high concurrency the risk is that moderation actions queue behind ordinary traffic.

When judging whether a peak was handled well, do not look only at whether connections stayed open. Ask whether moderation actions still reached clients promptly, and whether poll results updated on time. A fleet that holds many sockets but lags on moderation has not really coped. The autumn peak is a connection figure only. This note does not say how moderation latency behaved, and anyone who needs that must check the event's own monitoring.

## How to use this number

Quote it as: at the autumn summit, `pulse-socket` held 260000 concurrent connections across 18 nodes. Keep the two parts together. The connection count without the node count is easy to misread, and the node count without the connection count says nothing about load.

Do not compare it directly with the spring value unless you remember that the event size, audience behaviour and node count may have differed. The comparison that matters is that the new peak is higher than the old one, which was 210000 concurrent connections.

If you are writing a test or a load script, a target near this value is reasonable as a rehearsal goal. Set it above the peak, not at it, so the test shows where the system starts to strain.

## Open questions

Several things are not recorded here and should be filled in by whoever has the data:

- How much spare capacity the 18 nodes actually had at the peak moment.
- Whether any node was noticeably hotter than the rest.
- How long the peak lasted, and how quickly connections ramped up before it.
- Whether any clients had trouble reconnecting during the event.

Until someone answers these, do not treat the autumn run as proof that the same setup would hold a much larger audience. It shows the system worked at this level, and nothing beyond that.

## Housekeeping

This is a report, so it records a result and does not set a plan. If a decision follows from it, such as a new default node count for big events, put that in a separate decision note and point back here. When the peak is beaten, edit this note in place, change the value, and say which value it replaced, as was done here for the spring figure.
