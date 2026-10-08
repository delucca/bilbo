---
id: 01KYPGB2CA7QSGX7ZJ3C1KE986
created: 2026-07-29T05:37-03:00
---

# poll-engine option limit

This note replaces the earlier note "poll engine must accept"; the new value is that poll-engine now allows at most 20 options per poll.

## The rule

poll-engine now allows at most 20 options per poll. This replaces the earlier limit of 12 options. A poll with 20 options is valid. A poll with more than 20 options is still rejected, the same way larger payloads were rejected before. Only the ceiling moved; the rejection behavior did not change.

## Why it changed

Producers running large virtual events kept hitting the old cap of 12 options. Typical cases were polls that list every session track, every speaker, or every region. They had to split one question into several polls, which confused the audience and made the results harder to read. Raising the limit to 20 options covers those cases without turning the poll card into a wall of text.

## What stays the same

- Larger payloads are still rejected. The check is on the option count, and nothing relaxes it for any caller.
- Minimum option count is unchanged.
- Vote handling, tallies and the live result broadcast over WebSockets work as before.
- Moderation hooks still run on each option's text before a poll goes live.

## Where to look when touching this

The limit belongs to poll-engine on the Elixir and Phoenix side. The server is the source of truth. The Next.js client may show a counter or disable the add-option button, but it must not be the only place the limit is enforced. If the client and server ever disagree, the server wins and the client should show the server's rejection message.

Check that any validation, tests and docs still mention 12 options. Old fixtures that assume the former cap should be updated to the 20 options ceiling, and a boundary test should cover the exact maximum and one option above it.

## Storage and performance

Options are stored with the poll in CockroachDB. Going from 12 to 20 options adds a few more rows or a larger array per poll, which is small. Result broadcasts carry a slightly larger payload per update, so watch fan-out for very large audiences, but no change was needed when this was made.

## Open points

- Whether the UI should group or scroll a list of 20 options on small screens has not been decided.
- Existing polls created under the old cap are unaffected and keep working.
- Analytics exports may need a wider column set for polls that use the extra options.

## Related

For presence-related checks that share some of the same channel code, see [[presence-tracker-code-review]].
