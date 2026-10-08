---
id: 01M11FAJ25T275HVDSX7RKZ8GR
created: 2026-08-27T08:22-03:00
---

# poll-engine: general direction for vote handling and state

This note records the general direction the team picked for poll-engine. It is not a spec and it carries no tuned values. Limits, timeouts, batch sizes and similar settings live in config and in the heads of whoever last load tested, and they change too often to write down here. What is written here is the shape of the thing and the reasons, so the next person does not reopen it from scratch.

The short version: poll-engine owns the poll lifecycle and the tally. It keeps live state in Elixir processes, one per active poll, and treats CockroachDB as the durable record. Votes are accepted fast, acknowledged honestly, and counted in a way that can be replayed. The Phoenix channel layer is a thin edge and does not decide anything about a poll. The Next.js front end renders what the engine tells it and never computes results on its own. Moderation hooks sit in front of the tally, not behind it.

I wrote this in a hurry after the discussion settled, so some parts are blunter than they should be. If something here reads as wrong against the code, trust the code and fix the note.

## Why a process per poll

The main choice is that every active poll gets its own supervised process that holds the working state: the question, the options, whether voting is open, the running tally, and a small amount of bookkeeping about who has voted. Alternatives were considered. One was to keep all state in the database and do a read-modify-write on every vote. The other was a single big process or a shared in-memory table that serves all polls.

The database-only route was rejected for a plain reason. Large virtual events produce bursty traffic. When a host says "vote now", a huge share of the audience hits the same few rows at the same moment. A distributed SQL database is good at many things, but heavy contention on a handful of hot rows is the thing we most want to avoid, because transactions on those rows retry against each other and latency climbs right when the room is watching the screen. We would be building our worst case into the design.

The single shared process or table route was rejected because one noisy poll should not be able to slow down another. Events often run several polls at once, and a platform hosts several events at once. Isolation per poll gives us a natural unit for failure, restart and back pressure. If one poll process crashes, the supervisor restarts it, and the others do not notice.

So the process per poll is the working copy. It is fast because it is memory and message passing. It is not the source of truth, and that matters for everything below.

### What the process owns

The process owns the decision on whether a vote is accepted. That covers whether the poll is open, whether the option exists, whether this participant is allowed another vote under the poll's rules, and whether moderation has paused or closed things. It owns the running tally that gets broadcast. It owns the transition between states of the poll: draft, open, closed, and the results being published. These transitions are explicit and go one way, apart from a reopen path that the producer has to ask for deliberately.

The process does not own identity. Who a participant is comes from the session layer. The poll process receives an already-resolved participant reference and trusts it. That keeps authentication concerns out of the hot path and out of this component.

The process does not own rendering decisions. It emits state and events. How they are shown is a front end matter.

### Placement and lookup

Processes are found through a registry keyed by poll, so that the channel layer can route a vote without knowing where anything lives. We prefer to keep a poll's process on a single node at a time and route to it, instead of replicating live state across nodes. Replicating the live tally across a cluster was considered and dropped as too much machinery for the gain. If the node holding a poll goes away, the poll process is started again elsewhere and rebuilt from the durable record. The rebuild path is the thing we lean on, so it has to be good, and it is covered in the durability section.

There is a tradeoff here that we accepted knowingly: a short window where a poll is not being served while it moves. During that window clients are told to wait and retry, and they are not told the vote failed in any final sense. Better a brief pause than a wrong tally.

## Votes, acknowledgement and durability

The second big direction is how a vote travels and when we tell the participant it counted.

A vote arrives over the WebSocket from the Phoenix channel. The channel does cheap checks only: the message is well formed, the participant is joined to the right event, and the rate is not absurd. Then it hands the vote to the poll process. The channel does not touch the database for votes and does not keep tallies.

The poll process validates and decides. If accepted, it updates the in-memory tally and appends the vote to a buffer for persistence. Persistence is done in groups, not one write per vote, because writing each vote individually is exactly the pattern that makes a busy event expensive and slow. A separate writer path drains the buffer and writes to CockroachDB in batches, using idempotent inserts so that a retry after a transient failure cannot double count.

### What "accepted" means to the participant

We decided to be honest about the difference between "the engine saw your vote" and "your vote is durable". The participant interface shows the first quickly, because that is what keeps the experience feeling live. The durable state is what the final results are built from. We do not tell a participant their vote is permanently recorded in the same breath as the quick acknowledgement.

This is a deliberate softening. The alternative was to hold the acknowledgement until the write lands. That would make the interface feel sluggish under load and would tie participant latency to database latency. The team judged that the audience would rather see an immediate response, as long as the final result is correct. The cost is that there is a small exposure: if a node dies after acknowledging and before the write lands, some votes can be lost. We accepted that for ordinary polls and kept the exposure as small as practical by flushing promptly and on state transitions, especially at close.

Closing a poll is the important edge. When a poll closes, the process stops accepting votes, flushes everything pending, waits for the writes to be confirmed, and only then publishes the final result as final. Results shown while voting is open are labelled as live and may move. Results after close are the ones that go into any export or recap. If the flush cannot be confirmed, the poll is not marked final, and producers see that status instead of a quietly wrong number.

### Idempotency and replay

Every accepted vote carries an identity that is stable for that participant and poll and option choice. Writes use that identity so a repeated delivery is a no-op. This is what makes retries safe in every layer: the client resending after a reconnect, the channel resending to the process, the writer retrying a batch.

The tally in memory is considered a cache of the durable votes. The rule is that the tally can always be rebuilt by replaying the durable record. We keep periodic summaries so a rebuild does not have to read the entire history of a very large poll, but the summaries are an optimization and never a second source of truth. If a summary and the raw record ever disagree, the raw record wins and the summary is thrown away and recomputed.

We also decided not to build the engine around updating counters in the database directly. A counter column is simple, but it is the hot row problem again, and it cannot be audited. Votes as records plus a derived tally is slightly more work and much easier to defend when a producer asks why a number is what it is.

### Rules about who can vote and how often

Vote rules are a property of the poll: single choice, multiple choice, whether changing a vote is allowed, whether voting is open to everyone in the event or restricted to some group. The engine enforces these in the poll process, not in the front end. The front end may hide buttons, but hiding is only presentation.

Changing a vote, where allowed, is handled as a new record that supersedes the previous one for the same participant, not an in-place edit. That keeps the history intact and keeps replay simple. The tally derivation takes the latest relevant record per participant according to the poll's rules.

Anonymous polls are a presentation and export matter more than a storage matter. The engine still needs to know a participant voted, to enforce the rules. What changes is what can be read back out and shown to producers. The direction is to keep the data that enforces the rules separate from anything that could tie a choice to a person in exports, and to be conservative when in doubt. If a producer needs a specific anonymity guarantee for an event, that gets raised with the team and not assumed.

## Fan-out, moderation and the front end

The third direction covers what goes out to the audience and how moderation fits in.

### Broadcasting results

Pushing a fresh tally to every connected client on every single vote would waste a lot of work and flood the sockets. The decision is to broadcast on a cadence, not per vote: the poll process notes that its tally changed, and a periodic tick sends the current state to the event's subscribers if anything changed since the last send. The cadence is a tunable and is deliberately not recorded here. The principle is that the audience sees smooth updates, the server does bounded work per poll regardless of vote rate, and a slow client cannot make the engine do more.

Messages to clients carry the whole current tally and not just the delta. Deltas are smaller but they make a missed message corrupt the display until the next reconnect. Full snapshots are self-healing: a client that drops a message is right again on the next one. Given that polls have a limited number of options, snapshots are cheap enough. If a poll ever has a very large option set, we would revisit this for that case instead of changing the general rule.

State transitions, such as open, close and results published, are sent immediately and not on the cadence, since those are the moments people are waiting for.

### Moderation sits before the tally

Real-time moderation is a core promise of the product, and in poll-engine it shows up in two ways. First, moderators and producers can change a poll's state: pause, close, reopen, hide results, or retire a poll entirely. Those commands go to the poll process through the same path as everything else and take effect in order with votes, so there is no race where a vote slips in after a close was issued and is counted.

Second, free text options and write-in answers, where a poll allows them, go through moderation before they become visible options. The engine holds a submitted option in a pending state and exposes it to moderators. It only becomes a votable option when approved. Rejected ones are kept for the record and never shown to the audience. We chose to put this gate in the engine and not in the front end because the front end is not trusted to enforce anything, and because the audit trail belongs next to the data.

Moderator actions are recorded with who did them and what changed. That record is part of the durable state. The goal is that after an event, a producer can reconstruct what happened to a poll without relying on anyone's memory.

### The Next.js side

The front end consumes the socket stream and a plain request path for initial load. On first load it asks for the current state of the polls in the event, then subscribes for updates. After that, snapshots replace what it shows. The front end is not asked to be clever: no tallying, no vote deduplication logic beyond avoiding obvious double submits, no deciding whether voting is open. It submits, it shows what the engine says, and it handles the engine's "wait" and "rejected" answers with plain messages.

Optimistic display of the participant's own choice is fine and expected, since the engine acknowledgement arrives quickly. If the engine rejects the vote, the front end reverts and says so. The wording should say what happened in everyday language and avoid showing internal reasons.

Server-rendered pages are used for the pieces that do not need to be live, like the event shell and the producer dashboard layout. The live parts hydrate and connect. We did not want poll results to depend on server rendering timing, because a stale rendered result is worse than a brief spinner.

## Failure behavior, tradeoffs and what we left open

The last direction is how the engine behaves when things go wrong, and what we have knowingly not settled.

### Failure stance

The general stance is to degrade in the direction of correctness. Under pressure the engine will slow acknowledgements, delay broadcasts, or tell clients to retry before it will show a number it is not confident in. A late tally is acceptable. A wrong tally in front of a big audience is the failure we are designing against.

Database trouble. If CockroachDB is slow or briefly unavailable, the writer path buffers and retries with backoff, and the poll process keeps accepting votes into memory up to a point. Past that point it starts to push back on clients instead of growing without limit. The point is a config concern. What matters as a decision is that the buffer is bounded and that hitting the bound produces visible, honest back pressure and not memory growth until a node falls over. We also decided that transaction retries from the database are expected and handled in the writer, since the database asks clients to retry certain conflicts as a normal part of its operation. They are not treated as errors to page someone about unless they persist.

Process crash. A crashed poll process is restarted by its supervisor and rebuilds from the durable record plus whatever was safely buffered. Votes that were only in memory at the time are the exposure described earlier. We tried to keep the rebuild boring and well tested, because it will run at the worst moments.

Node loss. Same as above but at larger scale: polls that were on that node are started elsewhere on demand when the next request for them arrives, or proactively if we can detect the loss cleanly. Clients reconnect, get a fresh snapshot, and carry on. We do not try to hand off live memory between nodes.

Network partitions between nodes are the case where we most want to avoid two copies of the same poll process serving votes at the same time. The direction is that a poll process must be uniquely owned, and that when ownership is uncertain the safer move is to stop serving that poll briefly and let the durable idempotent writes sort out anything that did get through. Because writes are keyed by stable vote identity, even a brief overlap should not produce double counting in the final result, though live tallies could differ between the copies for a moment. That is another reason the final result is built from the durable record and not from either live copy.

### Abuse and fairness

Vote stuffing and scripted voting are a real risk for public events. The engine applies per participant rules, and the layers in front apply rate limits. We did not decide to build heavy fraud detection into poll-engine itself. Signals that look suspicious are recorded so moderators and producers can see them and decide, for example to void a set of votes. Voiding is done by marking records, not deleting them, and the tally derivation honors the marks. That way a producer can undo a voiding if it was a mistake, and the audit story stays intact.

We chose not to silently discard suspicious votes. Silent discarding makes it impossible to explain a result later, and it punishes real people who happen to share a network. Visible flags plus a human decision fit the product better, since the people running these events want control and want to be able to justify it.

### Tradeoffs we accepted

Memory first, database second means the engine is more complex than a plain database app, and it means we own a rebuild path and a flush path that have to be right. We took that cost because the live feel is the product.

Honest acknowledgement semantics mean the interface cannot promise permanence at vote time. We took that cost because the alternative made busy events feel slow.

Snapshots instead of deltas cost some bandwidth. We took that for resilience to dropped messages.

Keeping moderation in the engine means the engine knows more about content than a pure vote counter would. We took that so that the rules are enforced in one place with one audit trail.

### Left open

A few things are still open and should not be treated as decided just because they are not above.

How long finished polls stay as live processes versus being parked and rebuilt on demand. The leaning is to stop the process soon after close and serve results from the durable record, but the cost of rebuilds for popular recaps has not been measured well enough to say.

Whether very large events should split a single poll's tally across several processes and merge them. For now one process per poll is enough in our judgment, and splitting adds complexity that we would only take on after seeing a real limit. If we do it, the merge must stay correct under replay.

How anonymity guarantees are described to producers. The engine can support stronger separation, but the product wording and the exact promises are not settled and need input from whoever owns the product side.

How cross-event or cross-poll analytics should read from the vote records without touching the live path. The leaning is a separate read path off the durable record, kept away from the engine's processes. Nothing is built yet.

Reopen behavior after a final result was published. It is allowed deliberately by a producer, but how the earlier published result is marked afterward, and what audiences see, needs a clearer rule.

### If you change this

If a change touches any of the following, talk to the team first and update this note: the idea that the durable record is the source of truth, the idempotent vote identity, the rule that moderation commands and votes are ordered through the same poll process, or the rule that the front end never computes results. Those are the load-bearing parts. Most other things here, like cadence, buffering, and rebuild strategy, are expected to evolve as we learn from real events, and changing them is fine as long as the load-bearing parts hold.
