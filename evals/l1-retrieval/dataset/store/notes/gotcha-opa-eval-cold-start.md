---
id: 01M2FNNZ1SF3N9S4B4Q173SF2T
created: 2026-09-14T06:58-03:00
---

# opa-eval-lambda cold start timeout with large bundle

On a cold start with a large bundle, opa-eval-lambda dies with the message `Task timed out after 15.00 seconds` before the first evaluation runs. No policy has been evaluated at that point. No result is written for the scan item, and no remediation ticket is produced for it. This note is about that one failure: what it looks like, how to tell it from other failures, and what to do when it shows up. It is a trap because the function looks healthy on every invocation after the first successful one, so the problem is easy to miss in a quick test.

The failure belongs to opa-eval-lambda only. The other pieces of AuditMesh (the DynamoDB tables, the Jira ticket creation, the scanner that collects configs) are not where it happens. People hit it, assume the scanner or Jira is broken, and lose time. Start by looking at the opa-eval-lambda logs.

A related note covers the scanner side of the time budget: [[config-scanner-must-finish-revised]]. Read it if the question is about how long a whole scan may run. This note stays on the evaluation function.

## What you see

The visible sign is a single line in the function logs: `Task timed out after 15.00 seconds`. It comes from the Lambda runtime, not from our Python code. That matters for debugging. Our own logging inside the handler never prints anything for the affected invocation, because the handler body is never reached or has not logged anything yet. If you search for an application-level error and find nothing, that is consistent with this failure, not a sign that the logs are lost.

Typical shape of an affected invocation:

- The invocation starts with a fresh execution environment. The init phase is part of the same clock as the first request in the way the timeout shows up for us, so the whole thing is reported as one timed-out task.
- The log stream shows the start marker, then a gap, then the timeout line, then the end marker with a failed status.
- There is no line from our code saying a bundle was loaded, no line saying an input was received for evaluation, and no decision output.
- The report line at the end shows the duration at the configured limit, and the memory used is often high compared with a normal warm run.

The caller sees an invocation error. Depending on how the caller was invoked (synchronous from the scan orchestration, or through an event source with retries), the visible effect differs. A synchronous caller gets a function error back. An event source may retry and hit the same wall again if the retry lands on another cold environment.

## When it happens

It happens on a cold start, and only when the policy bundle is large. Both conditions are needed together in the cases we have seen.

- Warm invocations of the same function with the same bundle work. If you re-run right after a failure and it succeeds, you probably landed on an environment that was already initialised.
- Cold starts with a small bundle work. Test fixtures and the minimal local policy set do not trigger it. This is why the problem does not show up in unit tests or in a quick manual check with a toy bundle.
- Cold starts with the full production-sized bundle are the ones at risk. Any event that creates new execution environments can expose it: a fresh deploy, a configuration change that recycles environments, a quiet period followed by a burst, or scaling out when many scan items arrive together.

Because of this, the failure tends to cluster. After a deploy, the first wave of scan items may fail in a group, then things settle as environments warm up. A single failed item in a quiet period is also normal for this pattern. Do not treat a cluster right after a deploy as a regression in the policies themselves until you have checked the cold start angle.

The failure is not tied to a specific policy, a specific cloud account, or a specific resource type. Do not look for the input that caused it. The input is never evaluated.

## How to confirm it is this problem

Work through these checks in order. Stop when one gives a clear answer.

- Open the logs for the failing invocation and look for the exact text `Task timed out after 15.00 seconds`. If the text differs, for example a different duration, or an error raised from Python, you are looking at something else and this note does not apply directly.
- Check whether the invocation was a cold start. The report line for the invocation includes an init duration field only on cold starts. If that field is present and the invocation timed out, that fits.
- Check whether any application log line from the handler exists for that invocation. If none exists, evaluation never began. This is the strongest signal for this gotcha.
- Compare with a warm invocation of the same function from shortly before or after. If the warm one finishes normally and fast, with no init duration, the contrast confirms the pattern.
- Check which bundle the function is configured to load. If it is the full production bundle and not a test one, the large bundle condition holds.

If the first three checks agree, you can call it. You do not need to reproduce it further. Reproducing means forcing a cold start with the large bundle, which is easy to do by publishing a new version or changing an environment setting so the environments are replaced. Do that in a non-production setup only.

## What it is not

Several other failures look similar from far away. Rule these out so the wrong team is not paged.

- A policy evaluation that runs too long. That would log the start of evaluation, and our handler logs at the start of each item. Here nothing is logged.
- A DynamoDB throttling or timeout problem. Those show up as client errors from the data access code, raised by our Python, with service error names in the message. The timeout line here has none of that.
- A Jira outage or rate limit. Ticket creation happens after evaluation. If evaluation never ran, Jira was never called.
- A malformed input or a bad config from the scanner. Those fail inside the handler with a message from our code.
- An out of memory kill. That ends with a different runtime message, not the timeout text. High memory use on a cold start can sit next to the timeout, but the message to match is the timeout one.

If you see the timeout text together with the cold start markers and no handler logs, it is this gotcha.

## What to do when it happens

The practical handling, in the order people usually need it.

- Re-run the affected scan items. A retry that lands on a warm environment works. Since no result or ticket was written for the failed item, a retry does not duplicate anything. Check that the item has no stored result before assuming it needs a retry, though; look at the results table for the item first.
- If many items failed in a group, wait for the environments to warm up and then re-submit the failed set rather than the whole scan. Work out the failed set from the logs, not from guessing.
- Do not raise a ticket against the policy authors. Nothing is wrong with the policies.
- Do not mark the scan as clean. Items that timed out have not been checked at all. A scan with timed out items is incomplete, and reports built on it must say so. This is the dangerous part for the cloud security teams who rely on the output: a missing result looks the same as no violations if someone reads only the tickets.
- Record in the scan run notes which items hit the timeout so that the follow up re-run can be checked against that list.

For a deploy that is going to recycle the environments, plan for the burst of cold starts. Warm the function with a few harmless invocations before real traffic arrives, and confirm they complete. A warm-up invocation that itself times out tells you the large bundle problem is live in that environment, and you should hold traffic until one completes.

## Things worth trying and their status

This section tracks the approaches people have raised. Status words are plain: tried, works, does not help, not tried. Add to it when you learn more.

- Retrying failed items: works as a workaround. Does not remove the failure.
- Warming the function after a deploy: helps in practice. Not a guarantee, since new environments can still appear later when load grows.
- Testing with the full-sized bundle before release: should be part of any change that touches the bundle or the init path. A test with a small bundle proves nothing about this failure.
- Looking at the init path of the handler module, meaning what runs at import time and at first use, for work that scales with bundle size: worth doing when changing the function. Note what you find here.
- Changing the function configuration to give the init phase more room: not tried as a recorded result in this note. If someone tries it, write down the setting changed and what happened.
- Anything that changes how the bundle is delivered to the function: not tried as a recorded result. Write the outcome here when it is.

Keep entries short and say what you actually ran. Do not record guesses as results.

## Notes for whoever changes this function

A few habits that keep this from coming back unnoticed.

- When you touch the bundle, check its size relative to the last known good one. Growth makes the cold start worse before anyone sees a failure.
- When you touch the handler module, look at what runs before the handler starts. Anything added there adds to every cold start.
- Always include a cold start run with the production-sized bundle in your pre-release check, and read the report line for the init duration field. Compare it with the previous release.
- Add the timeout text to any log search or alert rule that watches this function, so the failure is counted and not buried. Match on `Task timed out after 15.00 seconds` as written. A rule that looks only for application errors will miss it, since our code logs nothing for the affected invocation.
- When writing about a scan result, say whether any item timed out. Do not let an incomplete scan pass as a complete one.

## Quick triage list

For someone paged at a bad hour, short form:

- Is the log text `Task timed out after 15.00 seconds`? If no, this is not it.
- Is there an init duration in the report line? If yes, it was a cold start.
- Are there handler log lines for the invocation? If none, evaluation never started.
- Is the bundle the full one? If yes, the condition holds.
- Re-run the failed items, confirm results appear, and mark the scan as incomplete until they do.
- Write down what you saw and what you tried in the status list above.

## Open items

Things still unknown or unrecorded as of this note.

- How the failure rate changes with the bundle as it keeps growing. No measured trend is written down yet.
- Whether every kind of cold start is equally exposed, or only some. Observed so far: deploys and bursts after idle periods.
- Which fix to adopt for good. The workarounds above only manage the symptom.

When any of these gets answered, edit this note in place and remove the item from the list.
