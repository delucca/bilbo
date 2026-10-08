---
id: 01KDD7QQYD8ZV82QZR936FT5Z7
created: 2025-12-26T08:47-03:00
---

# poll-engine: stale pid after restart

Votes sent to a stale pid after a poll process restart fail. poll-engine logs `** (EXIT) no process: the process is not alive`. Fix: callers must resolve the pid through the Registry on every call and never keep it.

## Names

The component is called poll-engine. Older code, docs and chat may use other names for it, so search for all of them.

### Previous name: ballotbox

ballotbox was the previous name of poll-engine. The component is called poll-engine now. If you see ballotbox in an old branch, an old dashboard or an old message, it is the same component as poll-engine.

### Codename: tallyho

tallyho is the internal codename of poll-engine. It is not a separate service. Treat tallyho and poll-engine as the same thing.

## Symptom

A vote comes in over the WebSocket and the handler sends it to a poll process. Nothing is counted. The log has this line:

```
** (EXIT) no process: the process is not alive
```

The client may see no error at all, and the tally stays flat for that poll.

## Cause

Each poll runs as its own process under a supervisor. When the poll process crashes or is restarted, the supervisor starts a new one. The new process has a new pid. Any pid cached before the restart now points at a dead process.

### Where the stale pid comes from

- A pid stored in a channel or socket assign when the user joined.
- A pid held in a long-lived task or GenServer state.
- A pid passed along from an earlier lookup and reused later.

## Fix

Look up the pid through the Registry on every call. Do not store it between calls. The Registry entry is updated when the supervisor restarts the poll process, so a fresh lookup finds the live pid.

### Rules for callers

- Resolve by the poll key at the moment of the vote.
- Handle an empty lookup result: the poll may be mid-restart or closed.
- Do not cache the result in socket assigns or process state.

## Handling a miss

A lookup can come back empty for a short time during a restart. Treat that as a retryable condition with a small bounded retry, or return a clear error to the client. Do not fall back to an old pid.

## Testing

Write a test that starts a poll, kills its process, waits for the supervisor to restart it, and then sends a vote through the normal path. The vote should be counted and the log should not show the EXIT line.

## Notes for later

- CockroachDB holds persisted results, but live tallies are in process memory, so a restart can lose counts not yet written. Check that separately.
- Moderation actions that target a poll should use the same Registry lookup.
- The Next.js front end does not know about pids, so the fix is only on the Elixir side.
