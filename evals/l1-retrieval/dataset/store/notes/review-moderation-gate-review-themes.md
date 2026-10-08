---
id: 01KGV8JVHGJPDR6HSE2HP1PBVA
created: 2026-02-07T02:18-03:00
---

# moderation-gate: themes from recent reviews

These are working notes on what keeps coming up when people review moderation-gate. They are general on purpose. They are not a list of findings and they record no decisions. They are meant to tell the next reviewer where to look first and what reviewers usually disagree about. moderation-gate sits between audience submissions and what everyone else sees. Questions, poll comments and chat-like messages all pass through it before they reach the stage view or the public feed. Most review comments are about timing, ordering and what happens when something is half done.

The reviews cover the Elixir side, the Phoenix channel layer, the CockroachDB storage and the Next.js moderator console. The themes below are grouped by where they show up, not by how bad they are. Some are old and keep returning in new forms. Check whether a theme is already handled before you open a new thread about it.

## Where the gate sits in the flow

Reviewers keep asking the same basic question: is moderation-gate a hard gate or a soft one? In other words, can a submission become visible to anyone before a decision exists for it? The answer in the code is mostly yes, it is hard, but the reviews keep finding side paths. A replay for a late joiner, an export, or a debug view can read from a table or a cache that holds pending items. None of these was a deliberate bypass. Each one came from a feature built later that read the data without going through the gate.

The habit that helps is to trace every reader of submission data and ask what state it expects. If a reader does not filter on state, treat it as a finding even if nothing is leaking today. Some reviewers want the gate to be the only module that can hand out displayable content. Others think that is too rigid for the way the code has grown. The disagreement is still open, so don't present either view as settled.

## State model and transitions

A second theme is the state model for a submission. The states are easy to name: pending, approved, rejected, held, and some variations. The transitions are less clear. Reviews point out transitions that exist in code but not in any written description, and cases where two moderators reach conflicting states from different screens. The usual question is whether a rejected item can come back, and who is allowed to do that.

Reviewers also dislike states that mean two things. A held item can mean waiting for a senior moderator, or it can mean waiting for the author to edit. Those need different handling in the console and in notifications. When a comment says the state is ambiguous, check for this first. Also check that every transition writes an audit record. A few reviews found paths that change state and leave no trace, which hurts later when a producer asks why something was shown.

## Concurrency between moderators

With many moderators working one queue, the most frequent worry is two people acting on the same item. Reviews ask what the second person sees. Some want an explicit claim on an item, so the console can show it as being handled. Others prefer optimistic writes where the later action loses and gets a clear message. Both approaches show up in the code in different places, and that inconsistency is itself a recurring review comment.

The database side matters here. CockroachDB gives strong guarantees, but transactions can be retried under contention, and reviewers keep checking that handlers are safe to run again. A retried handler that sends a notification or broadcasts to a channel inside the transaction is the classic problem. Side effects belong after commit. A few reviews also noted that hot rows, such as a counter of pending items per event, cause contention at exactly the busy moments when it hurts most.

## Real-time delivery and ordering

The Phoenix channel layer pushes decisions to viewers and to other moderators. Reviews raise ordering: can a viewer receive an approval before the item itself, or a rejection after the item has already been shown? The usual answer is that it can in rare cases, and the client has to cope. Reviewers want the client behavior written down, not left to whatever the frontend does by accident.

There is also the matter of fan-out. A decision that must reach a very large audience quickly is different from one that goes to a handful of moderators. Reviews ask whether the two use the same path and whether a slow subscriber can hold back the rest. Mailbox growth on busy processes comes up often. Reviewers tend to ask what happens to a connection that cannot keep up and whether dropping messages is acceptable for each kind of message. For approvals it probably isn't. For typing indicators it probably is.

## Failure and restart behavior

The supervision layout gets attention because moderation-gate holds state that is expensive to lose. Reviewers ask what is in memory only and what is durable, and what a crash does to items that were mid-decision. The preferred direction in comments is that memory is a cache and the database is the truth. In places the code still keeps queue ordering or claims in process state, and reviewers flag those spots.

A related question is what the gate does when it cannot decide. If the rules engine or a classifier is slow or down, does it fail closed and hold everything, or fail open? Reviewers mostly lean toward closed for public content, with a visible indicator for moderators that the queue is backing up. They also ask for a degraded mode that is explicit rather than a side effect of timeouts. Nobody has argued that failing open silently is fine, but some paths behave that way today.

## Automated filtering and rules

Much of the volume is handled by automatic checks before a human sees anything. Reviews on this part are about explainability and tuning. When an item is auto-rejected, can a moderator see which rule did it? Can a producer change the rules for one event without a deploy? The answers are partly yes. Reviewers want more of the reasoning stored with the decision, because disputes after an event depend on it.

Another concern is rule order and interaction. Rules written separately can contradict each other, and the outcome depends on evaluation order that is not always obvious. Reviewers ask for tests that pin down the order for the common cases. They also worry about rules that are cheap to write and expensive to run, since the gate sits on the hot path. Nobody wants a producer's pattern to slow submissions for the whole event.

## Moderator console in Next.js

The console gets its own set of comments. The main one is that the moderator needs to trust what is on screen. Stale lists, items that vanish while being read, and actions that appear to succeed but did not are the complaints that matter most. Reviewers ask for clear pending states on buttons and for a visible difference between my action is in flight and the server confirmed it.

Keyboard speed and bulk actions come up too. During a busy session moderators work fast, and reviewers care about misclicks on destructive actions. There is a tension with undo: a long undo window keeps content in limbo, a short one punishes mistakes. This is a product question as much as a code one, and reviews usually pass it back to the product side without resolving it. Accessibility and screen layout on smaller laptops are mentioned less often, but they are consistent.

## Permissions and roles

Role handling is a steady theme. Event producers, community managers, volunteer moderators and sometimes outside partners all touch the gate with different powers. Reviewers check that every action is authorized on the server and not only hidden in the console. They also check that a role is scoped to the event it was granted for. Cross-event leakage is the failure everyone is trying to rule out.

Temporary access is a pain point. Volunteers join for one session and are rarely removed afterward. Reviews ask for expiry that is enforced by the backend and for an audit view of who did what. They also ask how a removed moderator's open claims are released. This was missed in the past, and a stuck claim looked like an item nobody could handle.

## Observability and audit

Reviewers want to answer simple questions after the fact. What did the gate decide about this item, who or what decided it, and how long did it wait? The metrics and logs cover some of this but not all of it. Queue depth and wait time are the signals people want on a dashboard during a live event, and reviews note that the signals are sometimes aggregated in ways that hide a single stuck event among healthy ones.

Logging content is a sensitive area. Submissions can contain personal data or abuse, and reviewers push back on writing raw text into general logs. The preference is to log identifiers and decision metadata and keep the text in the controlled store. Traces across the channel layer, the gate and the database are called out as hard to follow, and better correlation between them is a recurring request.

## Testing and rehearsal

Test coverage comments are fairly uniform. Unit tests on individual rules are decent. Tests of the interactions are thin: two moderators racing, a restart in the middle of a decision, a burst of submissions right as a rule changes. Reviewers ask for property-style tests on the state transitions, and for load rehearsals that look like a real event, with bursty traffic around moments when a speaker invites questions.

There is also the point that rehearsals should include the moderator side. Load tests that only hammer submissions miss the case where moderators fall behind and the queue builds. Reviewers keep saying the real risk is a backlog that moderators cannot clear, not raw throughput. A rehearsal that shows how the gate behaves while behind is worth more than one that shows peak intake.

## Open disagreements and how to use these notes

A few questions stay open across reviews. How strict should the single path to displayable content be? Should claims on items be explicit or should optimistic conflict resolution win? How much of the decision reasoning should be stored? How long should an undo last? None of these has a recorded answer here. If you need one, raise it with the people who own the product behavior and write the outcome down in its own note.

For your own review of moderation-gate, a short order of work: list readers of submission data first, then walk the state transitions, then look at retries and side effects, then read the console for stale views. Leave the rules engine and permissions for last unless the change touches them directly. Treat repeat comments from earlier reviews as a hint about where the code is fragile, and check that a fix has not been quietly undone by a later change.
