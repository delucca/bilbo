---
id: 01K8JF7C18A5TXAQMNHJV0RVVX
created: 2025-10-27T06:15-03:00
---

# results-ledger: options survey

Quick survey of the general ways we could build results-ledger, the part of TownHall Pulse that holds poll tallies and Q&A vote counts so producers and community managers see the same numbers the audience sees. Nothing here is settled. It is a map of the options and the trade-offs I keep running into.

## What the ledger is for

Live polls and Q&A in big virtual events produce a lot of small writes in bursts. Moderation actions (hide, merge, pin) also change what counts. The ledger has to give a believable tally, show it fast over WebSockets, and still be explainable after the event.

## Options at a glance

There are four families: counters in memory, an append-only event log, direct row updates in CockroachDB, and a hybrid. Each is described below with what worries me.

## In-memory counters in Elixir processes

One process per poll or question set holds the tally and pushes changes out through Phoenix channels. Reads are cheap and fan-out is easy.

### Upsides

Very low latency. Fits the OTP model well. Easy to batch broadcasts so clients are not flooded.

### Downsides

A crash or deploy loses state unless we rebuild it from something durable. Moving a process between nodes needs care. Auditing is weak because only the current number exists.

## Append-only event log

Every vote and every moderation action is a row, and the tally is derived by folding the log.

### Upsides

Full history, easy to replay, easy to correct after a moderation reversal. Idempotency keys can stop double votes on retry.

### Downsides

Folding on every read is costly for large audiences, so we need snapshots or a running aggregate next to it.

## Direct row updates in CockroachDB

A tally row per option, incremented in a transaction.

### Upsides

Simple to reason about. One source of truth, and survives node loss.

### Downsides

Hot rows. Many writers hitting one key causes contention and transaction retries in a distributed SQL store. Sharding the counter into several rows and summing on read is the usual workaround, which adds complexity.

## Hybrid: log plus cached aggregate

Write events durably, keep a live aggregate in memory, and rebuild the aggregate from the log or a snapshot when a process restarts. This keeps the audit trail and the fast read path.

### Open worries

Two copies of the truth can drift. We need a clear rule for which wins and a periodic reconcile.

## Moderation interactions

Hiding or merging a Q&A item changes tallies after the fact. A log handles this naturally with compensating events. Plain counters need a subtract path that is easy to get wrong.

## Idempotency and duplicate votes

Clients reconnect and resend. Whatever we pick needs a per-voter, per-poll uniqueness check that does not become the new hot spot.

## Ordering and consistency

Strict global ordering is probably not needed. Per-poll ordering is enough. The question is whether a producer's dashboard may lag the audience view slightly, or must match it.

## Fan-out to clients

Next.js front ends subscribe over WebSockets. Options are push every change, push on a timer, or push diffs. Timer-based batching looks safest under bursts.

## Batching writes

Buffering votes briefly in the process and flushing to the database in groups cuts contention. The price is a small window of loss on a crash, which we need to accept or cover with a write-ahead step.

## Snapshots and compaction

If we keep a log we need periodic snapshots per poll and a policy for archiving old events once an event has ended.

## Multi-region concerns

CockroachDB can place data near users. Counters spread across regions raise latency for writes that need consensus. Region-local aggregates merged later are one idea.

## Sketch of the hybrid flow

```text
vote -> channel -> ledger process -> event row (CockroachDB)
                         |
                         +-> live aggregate -> broadcast to clients
```

## Testing ideas

Replay a recorded burst against each option, kill a node mid-run, and compare final tallies with a fold of the log. Also test moderation reversals under load.

## Leaning, not a decision

The hybrid looks most promising on paper because it answers audit and speed together. I still want to see how bad contention gets in the pure-database option before dismissing it.

## Open questions

- How much loss on crash is acceptable for audience votes versus moderation actions?
- Do producers need exact tallies live, or only at close?
- Who owns reconcile jobs, and where do they run?
