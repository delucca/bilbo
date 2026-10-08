---
id: 01KTWZ9YPD4A6J7DGAHZDG68G7
created: 2026-06-12T00:51-03:00
---

# poll-engine spec: option limit per poll

poll-engine accepts at most 12 options per poll. A create request or an edit request that carries more than 12 options is rejected with HTTP 422. That is the whole rule, and the rest of this note says where it applies, what the response looks like, and what each part of TownHall Pulse should do about it. Anyone reading only this note should be able to answer: what is the maximum, what happens above it, and what happens at it.

The limit is part of poll-engine's contract. It is not a client convention and not a UI hint. The Phoenix side of poll-engine enforces it, and the Next.js dashboard also checks it, but the server answer is the one that counts. If the two ever disagree, the server wins.

## The rule

A poll has a list of options. The list may hold anywhere from the minimum allowed up to 12 options. A list of exactly 12 options is valid. A list of more than 12 options is invalid and the whole request is refused. poll-engine does not trim the list, does not keep the first 12 and drop the rest, and does not create the poll in a partial state. Nothing is written.

The response for a rejected payload is HTTP 422. The status is 422 and not 400, because the request is well formed and understood; it is the content that fails validation. The body is the usual validation error body that the rest of the API uses, with an error entry attached to the options field of the poll. The message names the limit in plain words, so a producer reading it in the dashboard knows what to change.

The limit counts options as submitted. Blank options that the normalizer would strip are stripped first, and the count is taken after that. Duplicate options are a separate validation problem and get their own error; they are not counted twice and they are not merged to bring the count under the limit.

What counts as an option:

- Each entry in the options list of a create payload.
- Each entry in the options list after an edit is applied, meaning the resulting set and not just the entries that were added in that edit.
- Each option carried over when a poll is cloned from an earlier poll in the same event.

What does not count:

- Votes or tallies. Only the options themselves are counted.
- The "other" or free text answer, if a poll is configured to allow it. That answer is not an option in the list and does not use up a slot.
- Options that were removed in an earlier edit and are no longer part of the poll.

## Where it is enforced

The check runs in the poll-engine validation step, before anything is handed to the database layer. The order of work for a create request is: decode the payload, normalize the option list, check the number of options against 12 options, check the other fields, then write. A payload that fails the option count never reaches CockroachDB.

The same check runs for edits. An edit that would leave a poll with more than 12 options is refused with 422 and the poll stays as it was. An edit that removes options and adds options in the same request is judged on the final list, so swapping options in a poll that already holds 12 options is fine as long as the result is not above 12 options.

The check also runs for clone and for import. When a poll is built from a template or a previous event, the same validation applies and the same 422 comes back if the source holds more than 12 options. The importer reports the failing poll and carries on with the remaining polls in the batch; a failing poll does not abort the batch.

The check does not depend on how the request arrived. The HTTP API and the WebSocket channel commands that create or edit polls share one validation function. A command sent over the socket with too many options gets an error reply on the channel that carries the same 422 code and the same error body shape as the HTTP response. The socket stays open after that reply.

## Response shape and client behavior

On the server, a rejected request returns status 422 with the validation error body. No poll identifier is returned, no event is broadcast to the audience, and nothing is announced to moderators. From the point of view of everyone else in the event, the request did not happen.

On the Next.js dashboard, the poll builder should stop the producer from adding options past 12 options. The add-option control becomes disabled when the list reaches the limit, and a short note beside it says the limit has been reached. If a request still goes out above the limit, for example from a stale form or a pasted list, the dashboard shows the server message next to the options field and keeps the form contents so nothing has to be retyped.

Pasting a long list into the builder should fill options in order up to the limit and then show a message that the remaining lines were left out. That is client behavior only. The client does not send the extra lines, and the server still refuses a payload that has them.

Scripts and integrations that call the API directly should treat 422 on a poll payload as a permanent failure for that payload. Retrying the same body will give the same answer. The fix is to send a list of 12 options or fewer.

Things a client must not do:

- Do not retry a 422 automatically with the same payload.
- Do not split one poll into several polls to get around the limit unless the producer asked for that.
- Do not treat 422 as a server fault in alerting. It is an expected answer to a bad request and should be counted as a client error.

## Interaction with live polls and moderation

The limit applies at every stage of a poll, including while it is live. A producer may edit a live poll, and the edit rules above hold the same way: the final list must hold 12 options or fewer. Because votes are attached to options, removing an option from a live poll follows the existing rules for votes on removed options and is outside this note. This note only says that the count after the edit may not go above 12 options.

Moderation does not change the limit. A moderator who edits a poll sent in by a community manager is bound by the same maximum. There is no role that can exceed 12 options, and there is no per-event override. Staff accounts, admin accounts and service accounts all get the same 422.

The audience side is not affected. Audience members see the options of a poll as published and never submit option lists, so the check never fires for them. Results broadcasts over WebSockets carry as many tallies as there are options, which is at most 12 options.

The result views in the dashboard and in the embeddable display should be laid out for a full list of 12 options without scrolling inside the results panel on a normal desktop viewport. On narrow screens the list may scroll. This is a layout expectation for the Next.js code, not a server rule.

## Storage notes

CockroachDB stores options as rows tied to the poll. The database does not carry its own constraint for the maximum; the application check is the only gate. Because of that, any code path that writes option rows without going through the poll-engine validation step is a bug and should be reported. Direct writes from maintenance scripts need to apply the same check by hand.

Existing polls that were created before the rule was introduced and hold more than 12 options are not rewritten. They stay readable and they can still be run and shown. Any edit to such a poll, though, is judged on the resulting list, so an edit has to bring it to 12 options or fewer before it is accepted. The edit screen should tell the producer how many options have to go.

## Test expectations

Tests for poll-engine should cover the boundary on both sides. A payload with exactly 12 options is accepted. A payload with one more than that is refused with 422. The tests should check that no rows were written and no broadcast went out after a refusal.

Cases to cover:

- Create with the maximum number of options: accepted.
- Create with one option above the maximum: refused with 422, nothing stored.
- Create with a long list where blank entries bring the count under the limit after normalizing: accepted.
- Create with a long list where duplicates are present: refused for the duplicates, and the error is not reported as an option count error unless the count is also over.
- Edit that adds options to a poll already at the maximum: refused with 422, poll unchanged.
- Edit that adds and removes options so the final list sits at the maximum: accepted.
- Clone of a poll that is at the maximum: accepted.
- Import batch with one oversized poll among valid ones: the oversized poll is reported and the others are created.
- Channel command with too many options: error reply with the same code and body shape as HTTP, socket still open.
- Legacy poll above the limit: still readable, edit refused until the list is short enough.

The dashboard tests should cover the disabled add control at the limit, the paste behavior, and the display of a server 422 message next to the options field with the form contents kept.

## Open items

- The wording of the 422 message should be reviewed with whoever owns the dashboard copy. The status code and the field it attaches to are fixed; the sentence is not.
- The importer report format for refused polls needs a final look so producers can find the failing poll quickly.
- Decide whether API docs show the limit in the schema as a maximum on the options list, so generated clients pick it up. The stated limit is 12 options either way.
- Check that every place that builds a poll, including any older admin tool, goes through the shared validation step.

## Quick reference

poll-engine, options per poll: at most 12 options. Over the limit: HTTP 422, nothing written, nothing broadcast. At the limit: accepted. Same rule for create, edit, clone, import, HTTP and WebSocket commands. No role and no event can raise it. Server check is authoritative; the dashboard check is a convenience.
