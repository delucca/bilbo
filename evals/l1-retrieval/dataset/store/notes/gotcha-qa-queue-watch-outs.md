---
id: 01KWFHNXWWYZ55FK84TQZ18HGP
created: 2026-07-01T16:14-03:00
---

# qa-queue: things to watch out for when changing it

Quick list of traps in qa-queue, written from the point of view of someone about to change it. Nothing here is a spec. It is what tends to bite. Read the section that matches what you are touching, and skim the rest, because most breakages in qa-queue come from a change in one layer that quietly assumes something about another layer.

The short version: qa-queue looks like a list of questions, but it is really a shared, ordered, moderated, live-updated structure with many writers and a very large number of readers. Ordering, moderation state and delivery to clients all have to agree, and they are owned by different parts of the stack (Elixir processes, CockroachDB rows, Phoenix channels, the Next.js client). Change one without thinking about the others and the symptom shows up somewhere unrelated, usually in front of a big audience.

## Ordering, ranking and fairness

The order of questions in qa-queue is the product. Producers and community managers watch it, hosts read from it, and attendees judge the event by whether their question moves. So any change that touches how items are ranked needs more care than its diff size suggests.

Do not assume there is a single notion of order. There is usually arrival order, there is an upvote or popularity order, and there is whatever order a moderator has imposed by hand (pinning, pushing to the top, parking for later). These interact. A manual pin should survive a popularity re-sort. A question that was approved late should not jump ahead of older approved ones unless that is clearly intended. When you change the sort or add a new signal, write down which of these orders wins in each case before you write the code. If you cannot say it in a sentence, the behavior will be inconsistent between the server and the client.

Tie-breaking is the usual culprit. Two questions with the same score need a deterministic, stable tie-break, and it has to be the same everywhere that sorts: the database query, the in-memory structure in the Elixir process, and any sorting the Next.js client does for optimistic display. If any of these use a different tie-break, the list visibly flickers or reorders itself when a server update lands. Timestamps are a poor tie-break on their own, because many questions can arrive in the same instant during a burst and clocks on different nodes disagree slightly. Prefer a monotonic or database-assigned sequence for the final tie-break, and check that whatever you pick is actually assigned at insert time and not later.

Be careful with pagination and cursors. A cursor built on a score is unstable, since scores change while someone is paging. A cursor built on position is also unstable once items are inserted or removed above it. If you add or change paging, decide what the reader should see when the list shifts under them, and test that case on purpose. Duplicates and gaps across pages are the typical failure and both look like bugs to a moderator who is trying to clear the queue.

Fairness rules (per-person limits, rate limits on submitting, duplicate detection) live close to ordering and are easy to break. If you loosen a limit, think about what a single enthusiastic or hostile participant can now do to the front of the queue. If you tighten one, think about shared accounts, shared networks and kiosks, where many real people look like one. Duplicate detection in particular has a tension: merging near-identical questions helps the host, but merging wrongly hides someone's question. If you touch merging, make sure the merged items keep their combined support and that un-merging is possible or at least that nothing is silently lost.

Voting deserves its own warning. A vote is a write that can be repeated, retried, and raced. Make sure a change keeps votes idempotent per participant per question, that removing a vote is handled as well as adding one, and that a question being deleted or hidden does not leave stale vote counts that come back to life if the question is restored. Counters kept as denormalized values drift from the underlying rows if any write path forgets to update them. If you add a new write path, search for every place the counter is maintained and for any job that recomputes it.

Finally, remember that ordering decisions are made under load. A change that is fine with a handful of questions can be too slow when the queue is large and busy, especially if it re-sorts the whole thing on every vote. Look at what happens per vote, per submission and per moderation action, and prefer incremental updates over full recomputation in the hot path.

## State, processes and concurrency on the Elixir side

qa-queue state is held and mutated by Elixir processes, and the BEAM makes it very easy to write code that is correct for one caller and wrong for many. Most gotchas here come from assuming that a function call is atomic when it is really a message exchange.

First, know which process owns the authoritative copy of the queue for an event, and treat everything else as a cache. If you add a field, a status or a derived value, decide where it is computed and make sure there is a single writer. Two processes that both think they own the ordering will eventually disagree, and the disagreement tends to show up only at scale, when a node restarts or a process is rebalanced.

Be wary of read-modify-write across messages. A pattern like asking the owner for the current state, deciding in the caller, then sending an update back is a race. Between the read and the write another moderator may have approved, deleted or reordered the same item. Put the decision inside the owner, or use a conditional update that fails when the state has moved, and make the caller handle that failure in a way that makes sense to a person (refresh and show what changed rather than a generic error).

Mailbox growth is a real risk. A busy event can send a flood of votes and submissions to one process. If you add any slow work inside the handler for those messages (a database round trip, a call to another service, a profanity or spam check that is not trivial), the mailbox grows, latency climbs, and every other message waits behind it, including moderation actions that people need to be instant. Keep the hot handlers short. Push slow work to a separate process or task, and think about what the queue shows while that work is pending. Also think about back-pressure: what happens when the owner cannot keep up. Dropping silently is bad, blocking everything is bad, and the right answer depends on the message type. Votes can often be coalesced; moderation actions never should be lost.

Timeouts need thought. A call with a default timeout that is fine in tests may fail in a crowded event, and a failed call can leave the caller unsure whether the action happened. If an action is not idempotent, a retry after a timeout can apply it twice. Make actions idempotent where you can, for example by carrying a client-generated request token and ignoring repeats, and be explicit about what the caller should do on a timeout.

Supervision and restarts matter more than they seem. If the owner process crashes and restarts, it rebuilds state from somewhere, normally the database. Anything that lived only in memory is gone. When you add in-memory-only state (a temporary hold, a rate counter, a pending batch), decide whether losing it on restart is acceptable. If a restart can bring back questions that a moderator had already removed, or forget a pin, that is a visible bug. Also check what happens during a deploy with nodes of mixed code: messages and state shapes sent between nodes must be compatible in both directions for the duration of the rollout. Adding a field to a message or a struct is the classic way to crash the old nodes, or the new ones.

Be careful with process registration and event lifecycle. An event can be started, paused, ended and reopened. Code that assumes the queue process exists, or does not exist, at a given moment will fail in the reopen and late-arrival cases. Questions can arrive just after an event is closed, and moderators can act on a queue after the audience has gone. Decide what each of those should do and keep it consistent across the submit, vote and moderate paths.

PubSub and broadcasting from inside the owner are a common source of slowdowns. Broadcasting a full copy of the queue on every change is simple and expensive. Sending a diff is cheaper and easier to get subtly wrong, because a missed diff leaves a client permanently out of sync until something forces a resync. If you change what is broadcast, see the delivery section below.

Lastly, do not use unbounded data in process state. Queues for very large events can grow, and keeping every historical item, every vote and every audit entry in memory will eventually hurt. Keep the working set bounded and let history live in the database.

## Delivery over WebSockets and Phoenix channels

The people looking at qa-queue are connected over WebSockets, and the connection is not reliable. Treat every client as one that will miss messages, reconnect, receive messages twice, and receive them out of order. Any change to what the server pushes needs to be checked against those four.

On reconnect a client must be able to get back to the correct state. If your change relies on the client having seen every previous message, it will break the first time someone's laptop sleeps or a mobile network drops. The safe pattern is to give the client a way to ask for the current state and to include enough information in each update (a version or sequence) that it can tell when it has missed something. When you add a new kind of update, add it to that resync path too. It is easy to remember the live push and forget the snapshot, so a freshly joined client sees something different from a long-connected one.

Ordering of messages is not guaranteed in the sense you might want. An update about a question can arrive before the message that created it, particularly across reconnects or when different code paths publish. The client needs a defined behavior for an update to an item it does not have, and for a delete followed by a stale update. Without that, deleted questions reappear. Reappearing deleted or rejected questions on a public-facing screen is among the worst things this component can do, so test the sequence delete then late update explicitly.

Watch the audience split. Moderators, hosts, and attendees see different views of the same queue. Rejected, pending and hidden items must never be pushed to attendee channels. When you add a field to a payload, ask who receives it. A field that is harmless for moderators, such as the author identity, a moderation note, a flag reason or an internal score, can leak to everyone if the payload is shared. Prefer building separate payloads per audience instead of filtering on the client. Filtering on the client is not a security boundary, since anybody can read what arrives over the socket.

Authorization has to be checked per action, not only at join. A person who joined as an attendee should not be able to send a moderation message just because the channel accepts the event name. When you add a new inbound event, check who is allowed to send it, and check that permission changes (a moderator removed mid-event) take effect on connections that are already open.

Fan-out cost is the other big one. A large event means a huge number of subscribers, and a change that triples the payload size or doubles the number of messages per action multiplies across all of them. Before adding fields, think about size. Before adding a new broadcast per action, think about whether it can be merged into an existing one. Batching and throttling updates to attendees is normal and good: most attendees do not need every vote tick in real time, while moderators often do. Keep those two rates separate and do not accidentally apply attendee throttling to moderator screens, or the reverse.

Backpressure on slow clients matters as well. A client that cannot keep up can cause buffering on the server. Do not add anything that makes the server wait on a slow client in a shared code path.

Version skew between server and client happens. Tabs stay open for hours during long events, and the page loaded before a deploy may be speaking to a server after it. Payload changes should be additive and tolerant: new fields optional, unknown fields ignored, removed fields kept for a transition period. Renaming a field or changing its meaning without a transition will break open tabs in the middle of an event.

Heartbeats, idle timeouts and proxies can also interfere. If you change anything about how often messages are sent on a quiet queue, remember that an intermediate proxy may close idle connections, and that long silence is normal between questions.

## Persistence and CockroachDB

The database is distributed and serializable by default, and that changes how you should write queue code compared with a single-node setup.

Transaction retries are expected. Contended rows, for example a hot counter or the row holding an event's queue metadata, will cause serialization conflicts under load and the client library has to retry. Make sure any code you add inside a transaction is safe to run more than once: no side effects such as sending messages, broadcasting or calling external services inside the transaction body, because they will fire on every retry and may fire for a transaction that ultimately aborts. Do the side effects after commit.

Hot rows are the main design hazard. If every vote updates a single row (a counter per question, or a per-event total), a popular question or a big event turns that row into a bottleneck and a source of retries. Think about spreading writes, for instance by recording votes as separate rows and aggregating, or by batching updates in the owner process before flushing. If you add any aggregate that many writers touch, assume contention.

Monotonically increasing keys are another known pitfall in this kind of database: inserting with sequential keys concentrates writes on one range. If you add a table or an index for queue items or votes, consider the key choice and the write pattern. Likewise, an index that looks helpful for a moderation view can make every write more expensive, since each insert and update has to maintain it. Add indexes for a measured query, not a hunch.

Schema changes need care in a live system. Online schema changes run in the background and can be slow on large tables, and code from before and after the change will run at the same time during deploys. Add columns as nullable or with safe defaults, deploy code that can handle both shapes, and only later tighten constraints or remove old columns. Changing the meaning of an existing status value is worse than adding a new one, because old code will misinterpret it. If statuses are stored as text or as an enum, check every place that matches on them, including the Elixir pattern matches that crash on an unexpected value, the Next.js code that switches on them, and any reporting query.

Time handling: do not trust the application clock for ordering or for expiry decisions across nodes. Use database-side timestamps or sequences where order matters, and be explicit about time zones and precision. Comparisons between a value generated in Elixir and one generated by the database can disagree in ways that only appear in rare orderings.

Soft deletion is common for questions, because moderators need to undo. Every query that reads the queue must apply the same visibility rules. A new query written for a new screen or an export that forgets the deleted or hidden filter will expose removed questions. Centralize the visibility conditions rather than re-writing them, and when you add a status, review each query that selects questions.

Audit and history: moderation actions are often wanted afterwards, to answer who removed what and why. If you change how moderation is applied, keep the record intact. Do not let a refactor turn an update that wrote an audit entry into one that does not. Also consider that history tables grow without bound, and queries that scan them during a live event will compete with the live traffic.

Consistency between the database and the in-memory owner is a recurring topic. Decide what is written first. If memory is updated and the database write then fails, the screen shows something that will vanish on restart. If the database is written and the broadcast fails, clients are stale. Whatever order you choose, make failures visible and recoverable, and do not swallow errors in the persistence path just to keep the live path fast.

Read-heavy paths such as the attendee view should not hit the database per request during a big event. If you add a feature to the read path, check whether it is served from the in-memory state or from a cached snapshot, and avoid introducing a per-viewer query.

## Moderation behavior

Moderation is why this component exists next to simple Q&A, so any change must preserve the guarantees moderators rely on. The main one: when a moderator takes an action, it takes effect for the audience quickly and reliably, and it does not come back.

Status transitions need a clear model. Questions move between states such as submitted, pending review, approved, answered, rejected and hidden, and some events auto-approve while others require review. When you add or change a state, write out the allowed transitions, including the ones that go backwards (un-reject, un-answer, return to pending). Code that only thinks about the forward path will leave questions stuck or will let a late event (a vote, an edit) move an item back into a visible state it should not be in. In particular, make sure an edit by the author after approval does something deliberate: either it requires re-review or it is blocked, and that choice is consistent between the API, the channel and the UI.

Concurrent moderators are normal. Several people may work the same queue at once, and they will act on the same item. Last write wins is sometimes fine, sometimes not. If one moderator rejects a question while another is approving it, the result should be predictable and both of them should see the final outcome promptly. Show who did what where that helps, and avoid actions that silently overwrite a different moderator's decision without any indication.

Automated filtering (keyword lists, spam heuristics, rate-based holds) is part of moderation and has false positives. If you change it, think about how a held question is surfaced so that a real question does not vanish with no trace. A question held by automation should be visible to moderators with the reason, and its author should not be told something misleading. Be careful that automated rules apply on edits too, not only on creation, otherwise they are trivially bypassed.

Bulk actions are risky. Approve all, clear all, reject by filter: these touch many items and generate many broadcasts and many writes in one go. Check that they are bounded, that they apply atomically enough that the audience does not see a half-applied state for long, and that a mistake can be undone. Confirmation and undo belong in the UI, but the server also has to keep enough information to make undo possible.

Anonymous and named submissions need consistent handling. If a question can be asked anonymously, the author identity must not leak through any payload, export, log line or moderator-only field that can be forwarded elsewhere. Moderators sometimes need to see the identity for safety reasons, and that should be a deliberate, limited capability. When adding any new field or log, ask whether it carries identity, and check logs and error reports as well, since those often get wider access than the product does.

Privacy and retention: user-submitted text may contain personal data, and rejected content may be offensive or harmful. Do not copy it into places with weaker controls, such as metrics labels, verbose logs or analytics events. If you add telemetry for qa-queue, send counts and categories, not content.

After the event, the queue is often exported or archived for the organizers. Changes to status or visibility rules must be reflected in the export, which is usually a separate code path that is easy to forget. Check that rejected or hidden content is excluded or clearly marked there, the same as on screen.

Lastly, think about the moderator's workload under stress. A change that adds a step, moves a control, or changes the default sort of the moderation view can cost a lot during a live event, when people act quickly and from muscle memory. Prefer additive changes to the moderator view, and flag any change to defaults to the people who run events before it ships.

## Next.js client, testing and rollout

The client holds a copy of the queue and applies updates to it, often optimistically. That is where server and client assumptions meet, so a lot of subtle bugs live there.

Optimistic updates must be reconcilable. If the client shows a vote or a submission before the server confirms, it needs a plan for rejection, for a duplicate, and for the server returning the item in a different position than the client placed it. Rolling back cleanly is more important than looking fast. Make sure the client does not apply the same event twice, for instance once from the optimistic path and once from the broadcast, since that is the usual cause of double-counted votes and duplicated rows.

Rendering large lists is a performance problem. A big, busy queue can cause the browser to re-render constantly. Keep updates granular, avoid re-sorting the whole list on each message, use stable keys so items do not remount, and be careful with anything that triggers a layout of the entire list. Server rendering for the first paint is fine, but remember hydration: if the server-rendered snapshot and the first live state differ, the page can flash or warn. Anything time-dependent or random in the rendered output is a hydration risk.

Client state after reconnect: when the socket reconnects, the client should replace or merge with a fresh snapshot instead of continuing from stale local state. Test by dropping the connection during a burst of changes and checking that the end state matches what a freshly loaded page shows. This is the single most useful manual check for this component.

Accessibility and live regions: a list that updates constantly can be noisy for screen reader users. If you change how updates are announced, keep it quiet by default and meaningful when it speaks. Do not move focus on update. Moderators using the keyboard rely on stable positions, so an item should not jump out from under the cursor when the list re-sorts. Consider holding position for the item under focus or showing a clear indicator that new items are waiting.

Testing should cover what normal unit tests do not. Single-caller tests pass while the concurrent case fails, so write tests with multiple simultaneous actors on the same item: two moderators, a vote during a reject, a submission during a close. Include restart tests where the owner process is killed and rebuilt, and check that the rebuilt state matches. Include reconnect tests for the channel. Include a test that a payload for attendees never carries moderator-only data. Property-style tests are well suited to ordering and transition rules, since the number of combinations is large and hand-picked examples miss the odd ones.

Load behavior should be checked before merging anything that touches the hot paths: votes, submissions, broadcast, and the read path for attendees. A change can be correct and still make a large event unusable. Use a realistic mix, mostly viewers, a smaller group of voters, a few moderators, and bursts rather than a steady rate, because real events spike when a host asks for questions. Watch mailbox length, database retries, message sizes and client render cost, not just average latency.

Rollout deserves caution because the cost of a failure is public. Avoid shipping qa-queue changes right before or during a live event, and check what events are scheduled first. Prefer feature flags that can be turned off without a deploy, and where the change alters stored data, make it reversible: the old code should still work with the new data for as long as a rollback might be needed. Mixed-version periods are real, so server messages, database shape and client expectations all need to tolerate the neighbor version.

Observability: when you change behavior, make sure there is a way to see it working or failing in production. Useful signals are the lag between an action and its delivery, the rate of transaction retries, the size of the owner's mailbox, the number of resyncs clients ask for, and the count of items in each moderation state. A sudden rise in resyncs usually means the push path is dropping or reordering something. Do not log question text or identities to get this visibility.

Last, keep this note honest. When a change to qa-queue surprises you, or when one of the above turns out to be wrong or outdated, edit the relevant section here instead of adding a new note, so the next person starts from what is true now.
