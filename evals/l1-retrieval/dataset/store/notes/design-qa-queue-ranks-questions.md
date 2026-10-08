---
id: 01K2788HAS36RSW2AZHF9CGWC3
created: 2025-08-09T08:08-03:00
sources:
  - "code: lib/townhall/qa/ranker.ex"
---

# qa-queue ranking design

qa-queue orders audience questions by upvotes with a time decay. The half-life is 30 minutes, and the scoring lives in `lib/townhall/qa/ranker.ex`. This note records how the ranking is meant to work and why, so nobody has to rediscover it from the code.

## Purpose

Large virtual events get far more questions than a host can read. qa-queue has to put the questions the audience cares about near the top, and let newer ones get a fair chance.

## Ranking rule

Each question gets a score from its upvotes, reduced by how old it is. Higher score means higher position. Upvotes are the main signal; age only discounts them.

## Time decay

The decay is exponential with a half-life of `30 minutes`. After that time a question's upvote weight counts half as much as it did when it was posted. After two half-lives it counts a quarter.

## Why a half-life

Without decay, early questions collect votes for the whole event and stay on top. Newer questions about what the speaker just said never surface. A half-life keeps the queue tied to the current conversation.

## Where the code lives

All of the scoring is in `lib/townhall/qa/ranker.ex`. Keep the decay constant there and do not copy it into the Next.js client or other Elixir modules.

```elixir
# lib/townhall/qa/ranker.ex
# half-life: 30 minutes
```

## Inputs

The ranker reads the upvote count and the question's creation time. It does not look at question text or at who asked.

## Moderation interaction

Moderators approve, hide or answer questions in real time. Hidden and answered questions drop out of the visible queue. The ranker only orders what is left; moderation state is not part of the score.

## Real-time updates

Phoenix pushes queue changes to clients over WebSockets. When a vote arrives, the order is recomputed and the change is broadcast. Clients display the order they are sent and do no ranking themselves.

## Storage

Questions and votes are persisted in CockroachDB. Scores are derived values and are not the source of truth. They can be recomputed from votes and timestamps.

## Time drift

Decay depends on age, so scores change even with no new votes. Positions can shift when the order is recomputed. This is expected and not a bug.

## Known tradeoffs

- A fixed half-life suits some events better than others; a short panel may want faster decay.
- Old questions with many votes still sink eventually, even if the host wants them answered.
- Hosts can pin or answer questions by hand when the ranking is not what they want.

## Open questions

- Should the half-life be configurable per event?
- Should the decay be tied to event length?

## Testing notes

Test the ranker with fixed timestamps rather than the wall clock. Check that two questions with equal votes order by age, and that a question posted later can overtake an older one.

## Related

Moderation and poll code are separate components. Keep qa-queue ranking free of moderation logic.
