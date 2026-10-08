---
id: 01K4TA37CS755CYGQMQXBYB0A8
created: 2025-09-10T14:17-03:00
---

# verify-runner timeouts end in a cancel with no test report

When a job of the verify-runner goes over its time limit, GitHub Actions cancels it instead of failing it in the normal way. The log just stops, and the last line is `Error: The operation was canceled.` No test report is uploaded for that run. This is easy to misread, so it is worth writing down.

## Symptom

The upgrade pull request shows a red or cancelled check from the verify-runner job. Opening the log, there is no test failure, no stack trace and no summary of passed or failed tests. The log ends with `Error: The operation was canceled.` and nothing after it. The report artifact is missing from the run page.

## What it actually means

The error text says "canceled", which sounds like a person or a newer run cancelled the job. In this case it is usually the time limit being hit. The runner kills the step, so the step that would upload the report never gets to run. That is why the report is absent, not because the tests produced nothing.

If you see this message together with a missing report, treat it as a timeout first. Do not treat it as a test failure of the upgraded dependency.

## Why the missing report matters

PatchPilot decides whether an upgrade is safe from the test report. With no report there is no signal, so a cancelled run says nothing about the upgrade. Reading it as "tests failed" can make a good upgrade get rejected. Reading it as "no failures found" can make a bad one look fine. Neither is right.

## How to confirm

Check the job's start and end times on the run page against the configured limit for the job. If the duration is close to the limit, it was a timeout. Also check whether the same job on a different PR for the same repository finished normally, and how long it took.

Also rule out a manual cancel or a concurrency rule that cancels older runs when a new commit lands. Those produce the same log ending, so the duration check is what tells them apart.

## Likely causes

- A targeted test selection that grew too wide, so far more tests ran than expected.
- A hung test, often waiting on a network call or a lock on the SQLite file.
- A cold Docker image pull or a slow dependency install eating the time budget before tests start.
- A runner that is just slower than usual under load.

## What to do

Re-run once to see whether it was a slow runner. If it times out again, look at the test selection and at which step is slow. Raising the limit is a last resort, since it hides hung tests. Where possible, upload partial results from a step that runs even when the job is cancelled or fails, so a timeout still leaves something to read.

## Open items

Nothing in verify-runner currently marks a timeout in a way the rest of PatchPilot can see. A later change should record "timed out, no report" as its own outcome, separate from "tests failed".
