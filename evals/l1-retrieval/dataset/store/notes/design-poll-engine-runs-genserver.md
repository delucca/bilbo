---
id: 01KPRJQR2NS4PQCNDXSW7MTTSJ
created: 2026-04-21T14:52-03:00
---

# poll-engine design

This note describes how poll-engine is put together and why. It is written for someone who has to change the vote path or debug a poll that behaves oddly during a live event. The short version: poll-engine runs one GenServer per open poll under a DynamicSupervisor named `PollEngine.PollSupervisor`, and each of those processes flushes batched votes every `250 ms`. Everything else in the design follows from those two choices.

## Purpose and scope

poll-engine is the part of TownHall Pulse that accepts votes on live polls, keeps running tallies, and pushes results to attendees, producers and moderators. It does not own Q&A, and it does not own moderation rules. It only decides how a vote becomes a durable record and a visible number.

The load is lopsided. Most of the time there are few open polls, but when a producer launches one in front of a very large virtual audience, a huge number of votes arrive in a short burst for that single poll. The design has to survive that burst without making a hot poll slow down other polls, and without losing votes when a node goes away.

The edge is Phoenix with WebSocket channels. Clients are Next.js apps. Storage is CockroachDB. poll-engine sits between the channel layer and the database.

## Process model

Each open poll gets its own GenServer. The processes are started on demand by a DynamicSupervisor named `PollEngine.PollSupervisor`. When a producer opens a poll, the open request asks the supervisor to start a child for that poll. When the poll closes, the child is stopped after a final flush.

Why one process per poll:

- A poll is the natural unit of contention. Votes for one poll must be tallied together, votes for different polls never need to be.
- A process mailbox gives us ordering and serialization for free. No locks, no shared mutable tables for the tally.
- A crash is contained. If one poll process dies, other polls keep running.
- It is easy to reason about lifecycle: open means a process exists, closed means it does not.

The supervisor is dynamic because the set of open polls is not known at boot. Children are not restarted blindly forever; see the failure section for what happens when a poll process dies mid-event.

Processes are found through a registry keyed by poll identity, so channel code never holds a pid. Channel handlers look up the poll process by key and send it a message. If the lookup fails, the poll is treated as not open and the client gets a clear rejection.

## Vote intake path

A vote arrives on a WebSocket channel in Phoenix. The channel does cheap checks first: is the user authenticated for this event, is the payload well-formed, is the option one that exists. It then casts the vote to the poll GenServer. It does not wait for the database.

The poll process does the authoritative checks that depend on poll state: is the poll still open, has this voter already voted, and does the poll allow changing a vote. These checks run against in-memory state, so they are fast and consistent with the order in which votes were received.

Accepted votes are added to a pending batch inside the process and the in-memory tally is updated at once. The client gets an acknowledgement that means received and accepted, not durably stored. This distinction matters when reading logs: an ack does not prove the row exists in CockroachDB yet.

Rejected votes are answered immediately with a reason the client can show. Duplicate submissions are idempotent from the voter's side: the same choice twice is not an error the user needs to see.

## Batching and the flush interval

Writing one row per vote straight to the database would turn a popular poll into a write storm. Instead each poll process accumulates accepted votes and flushes them together on a timer. The flush interval is `250 ms`.

The reasoning behind that value:

- Short enough that the durable state trails the live tally by a fraction of a second, so a crash loses very little.
- Long enough that a busy poll turns thousands of single votes into a handful of multi-row writes per second.
- Short enough that results shown to moderators, which are read from the flushed state in some views, feel live.

The timer is re-armed after each flush completes, not on a fixed wall clock grid. That means a slow database write pushes the next flush later instead of stacking up overlapping flushes. An empty batch skips the write entirely, so idle polls cost almost nothing beyond the timer message.

If you want to change the interval, treat it as a trade between database load and the window of loss on a crash. Do not lower it just because a dashboard looks laggy; check first whether the lag is in the broadcast path instead.

## Persistence in CockroachDB

A flush writes the whole batch in one transaction. Votes are stored with a uniqueness rule on poll and voter, so a retried batch cannot double count. This matters because CockroachDB can ask clients to retry transactions under contention, and the flush code is written so that a retry is safe.

Tallies are not the source of truth. The vote rows are. The in-memory tally in the process is a cache that can be rebuilt from the rows by counting. A stored aggregate may exist for fast reads, but if it ever disagrees with the rows, the rows win.

Because the data is spread across nodes, keys are chosen so that votes for one hot poll do not all land on a single range if it can be avoided. Hot-range behavior is the thing to look at first if flush latency climbs during a big event while CPU on the app nodes is fine.

Schema changes to the vote table need care. Flushes run continuously during events, so a migration that blocks writes will show up as growing batches in the poll processes and then as memory pressure.

## Broadcasting results

Results go out to clients over the same WebSocket infrastructure. The poll process does not push to every attendee itself. It publishes an updated tally to a topic for that poll, and the channel layer fans it out.

Broadcasts are coalesced with the flush cycle: at most one results update per cycle, carrying the latest tally. Sending a message per vote would swamp both the server and the browsers. Clients receive a snapshot, not a delta, so a missed message is repaired by the next one.

Producers and moderators see the same tally as attendees unless a poll is configured to hide results until it closes. In that case the process still counts, but the public topic stays quiet and only privileged topics get updates. Check the visibility setting before concluding that broadcasting is broken.

## Failure and recovery

The main failure cases:

- **Poll process crashes.** Votes in the current unflushed batch are lost, at most one flush interval of them. The supervisor starts a fresh child for the poll, which reloads state from the stored rows, rebuilds the tally by counting, and resumes. Clients that had received an ack for a lost vote may see their vote missing; the client re-sync on reconnect shows the true state.
- **Database unavailable or slow.** The flush fails or takes long. The process keeps the batch and keeps accepting votes, retrying on the next cycle. There is a bound on how large the pending batch may grow; past it the process starts rejecting new votes with a retry-later answer instead of eating memory.
- **Node goes away.** All poll processes on it die together. They are restarted on whichever node next receives a request for that poll. Because state is rebuilt from rows, no hand-off is needed, only the same small loss window as a process crash.
- **Repeated crash loop.** The restart intensity is limited. If a poll keeps crashing, the supervisor gives up on it rather than looping, and the poll shows as unavailable. That is deliberate: a poisoned poll should not take the node's supervision tree down with it.

The honest summary of the guarantee: accepted votes become durable within about one flush cycle, and a crash can lose the votes still pending in that cycle. If that is not acceptable for some product use, the answer is a write-ahead step before the ack, which this design does not have.

## Moderation interaction

Real-time moderation touches polls in a few ways. A moderator can close a poll early, remove an option that turned out abusive, or void votes tied to a banned account. These all go through the poll process as messages, so they are ordered with respect to votes.

Closing forces a final flush before the process stops, so a closed poll has complete stored data. Removing an option marks it hidden and recomputes the tally; stored votes for it are kept for audit instead of deleted. Voiding votes from an account adjusts the tally and records the change in the same flush.

Moderation actions never bypass the process to edit rows directly. Doing that would leave the in-memory tally stale until a restart and would be hard to explain later.

## Operating notes and gotchas

- Number of processes equals number of open polls. A forgotten open poll is a live process with a live timer. Closing polls at the end of an event is part of cleanup, and a stuck one should be visible in the supervisor's child count.
- An ack to a client is not proof of durability. When a voter says their vote vanished, check whether the poll process crashed around that time before suspecting the client.
- A single very hot poll is bounded by one process mailbox. If intake ever saturates a single poll, the options are cheaper per-vote work in the process or sharding the intake in front of it. Neither is built.
- Mailbox growth is the early warning sign. Watch queue length per poll process during large events, not just overall CPU.
- Tests that touch timing should control the flush rather than sleep; waiting for the real timer makes them slow and flaky.
- Changing the flush interval changes both database load and loss window. Do both analyses.

## Open questions

Things not settled and worth a decision before the next big event:

- Whether to add a durable log ahead of the ack for events where losing even a fraction of a second of votes is unacceptable.
- Whether a hot poll should be split across several intake processes that merge their tallies, and how that would interact with the one-vote-per-voter rule.
- Whether the batch bound should be configurable per event rather than global.
- Whether results visibility rules belong in poll-engine or in the channel layer; today they are split, which causes the confusion mentioned above.

## Where to look next

Start from the supervisor and the poll process module in the poll-engine code when tracing any behavior listed here. For database symptoms, look at the flush transaction and its retry handling first. For client-visible symptoms, look at the channel handlers and the broadcast coalescing. This note was written from the design as understood, not from a fresh read of the code, so confirm details there before relying on them for a change.
