---
id: 01KZB35VHPRSDZHXKKZN9GZ5XK
created: 2026-08-06T05:31-03:00
---

# Poll-engine load test

This note replaces the earlier note "poll engine load test". After the batching rewrite, a load test showed poll-engine sustaining 26000 votes per second, which replaces the earlier figure of 18000 votes per second. Use the new value anywhere the old one still shows up.

## Result

The current sustained throughput of poll-engine is 26000 votes per second. The previous figure, 18000 votes per second, was measured before the batching rewrite and is no longer valid. The gain comes from the rewrite and from nothing else that I changed during the test. The test ran against the Elixir and Phoenix service with WebSocket clients casting votes and CockroachDB as the store.

```text
poll-engine load test, after batching rewrite
sustained: 26000 votes per second
previous:  18000 votes per second (superseded)
```

If someone quotes the old number in a doc, a dashboard description or a capacity plan, correct it to the new one. I have not checked every place where the old figure might be written down, so expect a few leftovers.

## What changed

The rewrite made poll-engine group incoming votes into batches before writing them, instead of handling each vote as its own unit of work all the way to the database. The goal was to cut per-vote overhead in two places: the number of round trips to CockroachDB, and the amount of per-message work inside the Elixir processes that own a poll.

The client-facing contract stayed the same. A viewer casts a vote over a WebSocket and gets an acknowledgement. The tallies pushed to the audience and to the moderation view keep the same shape. What differs is the internal path between receiving a vote and having it durably counted.

The batching trades a small amount of delay for throughput. A vote can now wait briefly for its batch to fill or flush. That delay is short enough not to be visible to a person watching a live tally, but it exists, and it is the first thing to look at if someone reports that results feel laggy.

## How the test was run

The test used simulated viewers holding WebSocket connections and casting votes against active polls, ramped up until throughput stopped growing. The 26000 votes per second figure is the level the service held steadily, not a short peak. I count a level as sustained only when it was held for a meaningful stretch with no growing backlog.

The Next.js front end was not part of the load. The clients talked to the Phoenix endpoint directly, so the number describes the backend vote path and not the full browser experience. A real event adds page loads, reconnects and the Q&A traffic on top of this.

The load generator was driven from separate machines so it would not compete with poll-engine for CPU. I did not record exact hardware here on purpose. If this test is repeated, write the environment down next to the result, because a throughput number without its environment is hard to compare.

## What the number does and does not mean

It is a ceiling for the vote path under test conditions. It is not a promise for any event. Real events have uneven traffic: a presenter opens a poll and a large share of the audience votes within a short window. That burst can be much sharper than a steady ramp, so the burst shape matters more than the average.

It also does not cover moderation load at the same time. Real-time moderation reads and acts on the same data as voting, and a busy moderator team adds work. The test did not stress that overlap, so treat the figure as vote casting alone.

Finally, it measures one poll-engine deployment shape. Changing how many nodes run, how polls are placed across them, or how the database is laid out can move the number up or down. The new figure should not be extrapolated to a different shape without measuring again.

## Risks to watch

- Batch delay: the batching adds a small wait for each vote. Watch for user complaints about slow tallies.
- Failure inside a batch: when a batch write fails, several votes are affected together instead of one. The retry and error handling around a failed batch deserves a second look before a large event.
- Hot polls: one very popular poll concentrates writes on the same rows in CockroachDB. Contention there can cap throughput below the headline figure for that poll, even when the service as a whole has room.
- Reconnect storms: if many WebSocket clients drop and return at once, the resulting surge is not what this test measured.
- Ordering: batching can change the order in which votes land relative to when they were cast. For counting this does not matter, but it can matter for anything that depends on exact order, such as a poll closing at a precise moment.

## Follow-ups

- Repeat the test with moderation actions running at the same time and record the effect on throughput.
- Test a sharp burst on a single poll, since that is the realistic worst case for event producers.
- Record the environment and the exact load profile alongside the next result.
- Search the docs, dashboards and runbooks for the old figure of 18000 votes per second and update them to 26000 votes per second.
- Decide whether the batch size and flush timing should be configurable per event, so a producer running a smaller, interactive session can favor low delay over throughput.

## Where this note fits

This is a report on one measurement, not a design or a decision. It supersedes the earlier "poll engine load test" note, which should be treated as out of date. If a later test changes the figure again, update this note in place instead of starting another one, so there is a single place that holds the current number for poll-engine and a short trail of what it replaced.

The short version for anyone in a hurry: poll-engine now sustains 26000 votes per second after the batching rewrite, up from 18000 votes per second, measured on the vote path only, with burst behavior and moderation overlap still untested.
