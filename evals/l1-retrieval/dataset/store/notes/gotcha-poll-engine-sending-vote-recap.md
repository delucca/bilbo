---
id: 01KV2ZPM1WV0SD8KTKP83MCJ3G
created: 2026-06-14T08:54-03:00
---

# Poll engine sending vote: loose gotchas

Writing this down fast because the vote path in poll-engine keeps biting people, and I did not go back to check what was already noted. Treat it as a second pass on the same subject. Related survey of storage options is in [[results-ledger-options-survey]]; this note is only about what happens when a client sends a vote and the engine has to accept it.

The short version: a vote that looks accepted on the socket is not necessarily counted yet. The acknowledgement and the durable write are two different moments, and most of the surprises come from assuming they are the same one.

## What the send path looks like

A browser client built in Next.js opens a WebSocket to the Phoenix endpoint and joins a channel for the poll. When the participant taps an option, the client pushes a vote message on that channel. The channel process validates the poll state (open, not closed, not paused by a moderator), checks that the participant is allowed to vote, and then hands the vote to poll-engine for counting.

Inside poll-engine the vote goes through a per-poll process. That process keeps the live tallies in memory and also forwards the vote to the database layer on CockroachDB. The reply to the client goes out based on the in-memory step, not on the database commit. That is the root of most of what follows.

I did not trace every branch. Where I say "I think" below, I am working from memory of reading the code and not from a fresh run.

## Ack before commit

The client gets an ok reply as soon as the per-poll process has accepted the vote. At that moment the database write may still be queued or in flight. If the node dies between the reply and the commit, the participant saw success and the vote is gone. For a casual poll that is tolerable. For a poll where the producer reads results on stage, a few missing votes can be visible if the restart happens during a busy moment.

Things to keep in mind when changing this:

- Do not "fix" it by waiting for the commit inside the channel callback. The commit latency under load is much higher than what the channel can afford, and the channel mailbox backs up. We saw the effect once as laggy joins for everyone on the node, not only voters.
- If you need stronger guarantees for a specific poll type, make it an explicit mode, and say so in the moderation UI so producers know what they are choosing.
- The reply payload should not claim "recorded". Use wording that means "received". The client copy was changed once already and then drifted back.

## Duplicate and retried votes

Clients retry. A flaky connection on a large virtual event produces reconnects, and the Next.js side resends the last vote it did not get a reply for. The engine has to treat a resend as the same vote. The idempotency key is built from the participant and the poll, plus a client-side token for the attempt. If a client forgets the token, the engine cannot tell a retry from a new vote, and for polls that allow changing an answer that looks fine while for single-choice polls it silently overwrites.

Gotchas here:

- A retry that arrives after the poll closed must be rejected the same way a fresh vote would be, even if the first attempt was accepted before close. I think the engine currently answers with the already-accepted result in that case, which is correct, but it depends on the key being present. Check this if you touch the close path.
- Multi-select polls: a retry of a partial selection is not the same as a retry of the full selection. Keep the whole selection inside the key material, not only the poll and participant.
- Do not dedupe by socket id. Reconnects get a new one.

## Ordering with moderation actions

Moderators can pause, close, or remove an option while votes are arriving. The per-poll process serializes these, which is good, but the order is the order the process received them, not the order users clicked. A vote sent a moment before a close can legitimately land after it and be refused. Participants then see an error that feels unfair.

What helps in practice:

- Show a short grace behavior on the client: if the close event arrives right after a send, display a neutral message rather than a failure.
- Do not extend the grace window on the server unless the producer asks for it. It changes results after the fact and moderators notice.
- When an option is removed mid-poll, decide up front whether the votes already cast for it are dropped or kept in an archived tally. The engine does one thing by default and I do not remember which; check before promising anything to a producer.

## Backpressure and fan-out

Large events mean many voters sending around the same time, usually right after the host says "go". The engine batches tally broadcasts so that the channel does not push a message per vote to every subscriber. The batch interval is configured, and the usual value is fine for most events. Shortening it makes the live bars feel nicer and costs a lot of CPU and bandwidth on big audiences.

Points worth remembering:

- The vote intake and the results broadcast are separate concerns. Slowing the broadcast does not slow the intake, and it should stay that way.
- If the per-poll process mailbox grows, the symptom is that acks get slow first, then the tallies lag, then reconnects start, which adds more load. Watch the mailbox length before anything else.
- Rate limiting per participant exists, with a configured limit. Hitting it returns a refusal that the client should not retry immediately. An earlier client build did retry immediately and made it worse.

## Database side

The writes go to CockroachDB. Transaction retries are normal there under contention, and the write path is expected to handle them. The hot spot is many votes updating the same row, for example a single counter per option. The engine avoids that by writing individual vote rows and aggregating, but a shortcut that increments a counter will bring back contention fast. Please do not add one for convenience.

Reading tallies for a restart or for a late-joining viewer comes from aggregation plus the in-memory state. After a restart the in-memory tallies are rebuilt from stored votes, so any vote that was acknowledged but not committed is simply missing. This is the same gap as the ack-before-commit issue above, seen from the other side.

Whether a separate append-only ledger would help close that gap is what the survey note is about. I have no conclusion on it here.

## Testing notes

Things that were easy to get wrong when testing this path:

- Tests that send a vote and immediately read the tally from the database will be flaky, because of the ack-before-commit gap. Read from the engine state, or wait on the commit signal if there is one.
- Reconnect tests need to drop the socket without the client closing cleanly. A clean close hides the retry behavior.
- Load tests should run with the configured batch interval and limits, not with them turned off, or the numbers do not mean anything for a real event.
- Moderation races need a deterministic way to order messages in the per-poll process. Sleeping in the test is not it.

## Open questions

- Which side wins when a retry and a close cross: I believe the accepted vote stands, but I have not confirmed it against the current close handler.
- Whether the removed-option behavior is documented anywhere producers can see.
- Whether the client should show a pending state until the commit is known, and how that would be signaled back over the channel without making the channel wait.
- If a vote is lost on a node crash, is there any way for the participant to learn about it? Right now no.

If someone picks this up: start with the ack-before-commit gap, since the retry and moderation issues are easier to reason about once that is settled.
