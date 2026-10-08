---
id: 01K433EDQW30QZGKA77GHV7540
created: 2025-09-01T13:58-03:00
---

# qa-queue weekly recap

This week on qa-queue was mostly cleanup and tightening, with a few loose ends left open. Notes below are rough and written fast. Nothing here fixes a value or a final decision; treat it as a trail for whoever picks it up next.

## Where things stand

The queue still takes audience questions from the Next.js front end over WebSockets, passes them through moderation, and hands approved ones to the producer view. The basic path works. Most of the week went into the edges around it, not the core flow.

```text
audience (Next.js) -> Phoenix channel -> qa-queue -> moderator -> producer view
                                            |
                                       CockroachDB
```

## Ordering of questions

We spent time on how questions are ordered when many arrive at once. Upvotes and arrival time both matter, and the two do not always agree. We looked at a few cases and wrote down the odd ones. No ordering rule is settled yet. It needs more thought before anyone codes it.

## Moderation hand-off

The step where a question moves from pending to reviewed got cleaner. Moderators were seeing the same item twice in some cases. We traced it to how the hand-off is claimed and tightened that part. It looks better now, but it needs watching under a busy session.

## Duplicate questions

Audiences ask the same thing in different words. We discussed grouping near-duplicates so moderators do not review the same topic over and over. Still at the idea stage. The open point is whether grouping should be automatic or only suggested to the moderator.

## Persistence in CockroachDB

We reviewed how queue state is written. Some writes could contend when many moderators act on nearby rows. We adjusted a couple of access patterns and noted where retries are needed. More load testing would tell us if this is enough.

## Retries and transactions

Transaction retries are now handled in a more consistent way in the queue code. Before, a few call sites handled them and others did not. Not every site is covered yet, and the remaining ones are listed in the working scratch list.

## Real-time delivery

The Phoenix side pushes queue changes to connected clients. We looked at what happens when a client drops and reconnects mid-session. The client now asks for the current state on rejoin instead of trusting what it last saw. That fixed the stale views we saw earlier.

## Backpressure

When a popular session floods the queue, the channel processes can fall behind. We sketched ways to slow intake gracefully, with no change merged. The main worry is that audience members should not see their question silently vanish.

## Front end behavior

The Next.js side got small changes to how it shows a submitted question while it waits for confirmation. People were resubmitting because nothing visibly happened. A clearer pending state helps. Copy and styling are still rough.

## Moderator tooling

Community managers asked for faster bulk actions. We drafted what a bulk approve or reject might look like and who should be allowed to use it. Permissions are the hard part, so this stays on paper for now.

## Producer view

Producers want to see what is coming next without being buried in the full backlog. We discussed a narrower upcoming list. No build yet, only agreement that the current view shows too much.

## Testing

We added a few tests around the hand-off and reconnect paths, since those caused most of the surprises. Coverage on ordering is still thin. Property-style tests would suit it, and that is a good next task.

## Related note

The question of how to record and present outcomes touches this component too. See [[results-ledger-direction-chosen]] for the direction taken there. We should check that qa-queue does not assume anything that contradicts it.

## Risks

The biggest risk is load during a very large event, where ordering, persistence, and delivery all get stressed together. We have only tested the pieces separately. Another risk is that the duplicate grouping idea grows into something too big.

## Open questions

Should ordering be configurable per event? Should moderators be able to pin a question? How long should answered items stay visible to producers? None of these has an answer yet.

## Next week

Finish the retry coverage, add ordering tests, and run a heavier session simulation against the queue. Then revisit backpressure with real observations instead of guesses.

## Handoff notes

If you pick this up cold, start with the hand-off and reconnect code, since that is where recent changes landed. Keep changes small and re-check the moderator flow by hand after each one.
