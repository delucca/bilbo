---
id: 01K7JJD3S6HABA5FNGMGJ4NS09
created: 2025-10-14T20:55-03:00
---

# ticket-sync-worker weekly recap

Short recap of the week on `ticket-sync-worker`. Most of the time went into making the Jira side less surprising and into reading how the DynamoDB state behaves when the same finding shows up twice. Nothing here is final; it is what I would want to know on Monday morning.

## What the worker does right now

`ticket-sync-worker` runs as an AWS Lambda. It picks up violations that Open Policy Agent has already flagged, looks them up in DynamoDB to see whether a ticket exists, and then creates or updates a Jira issue. The Python code is small. Most of the trouble is in the edges: retries, duplicate input, and Jira being slow or rejecting a field.

## Work this week

- Went through the create path and the update path side by side. They share less code than I expected, and a few fields are built twice with slightly different rules.
- Traced a case where one finding arrives from two scans close together. Both invocations see "no ticket yet" and both try to create one. This is the main source of duplicate tickets people have complained about.
- Tidied the logging so a single invocation can be followed from the incoming finding to the Jira response. Before, you had to guess which log lines belonged together.
- Started separating the Jira client from the sync logic so the client can be faked in tests. Not finished.
- Read through how failed invocations get retried, to see whether a retry after a partial success can create a second issue.

## Things that bit me

The lookup-then-create sequence is not atomic. Reading DynamoDB and then writing the ticket reference afterwards leaves a gap, and two invocations can fall into it. A conditional write when claiming the finding looks like the right shape, but I have not tried it against real traffic.

Jira validation errors are not all the same kind. Some mean the payload is wrong and retrying is pointless; others are transient. The worker currently treats too many of them alike, so bad payloads get retried and good ones sometimes get dropped.

Local runs against a stand-in for DynamoDB hide the race entirely, because everything is serial. A passing local test says little about duplicates.

## Rough shape of the claim idea

Sketch only, not code that exists in the repo:

```python
# claim the finding first, create the ticket second
claim_finding(finding_key)    # conditional write, fails if already claimed
issue = create_jira_issue(finding)
save_ticket_ref(finding_key, issue)
```

The open question is what happens when the claim succeeds and Jira creation then fails. A claim with no ticket behind it needs a way to expire or be retried, otherwise the finding is stuck.

## Next week

- Finish pulling the Jira client out so the sync logic can be tested without the network.
- Write a test that fires two overlapping invocations for the same finding and checks that only one ticket results.
- Sort Jira errors into retry and do-not-retry groups and make the worker follow that.
- Decide how a stale claim gets cleaned up, and ask the people who read the tickets whether they would rather see a duplicate or a short delay.
- Check that the log changes do not leak anything sensitive from the finding contents.

## Open questions

Does anyone depend on the current update behavior of overwriting fields that humans edited in Jira? I saw hints of it in the update path but did not confirm. If so, the update path needs to be more careful before we change anything else.
