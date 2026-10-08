---
id: 01KDCB2HC10TJXTZ0CR254YC3C
created: 2025-12-26T00:26-03:00
sources:
  - "code: .github/workflows/verify.yml"
---

# verify-runner design

The verify-runner is the part of PatchPilot that checks an upgrade pull request after it has been opened. It does not run tests inside the PatchPilot process. It hands the selected tests to a GitHub Actions workflow and reads the outcome back. This note records how that works and why it was set up this way.

## Purpose

PatchPilot opens dependency upgrade pull requests across many repositories. A bump is only useful if someone can trust it, so each pull request gets a targeted test run. The verify-runner owns that step: it takes the list of tests chosen for the change, runs them, and reports pass or fail for the pull request.

## Where the tests run

The verify-runner executes the selected tests in the workflow `.github/workflows/verify.yml`. That workflow is started through the `workflow_dispatch` event. So the trigger is an explicit dispatch from PatchPilot, not a push or pull request event. The workflow file lives in the target repository, and the runner only dispatches it and waits.

## Why workflow_dispatch

Using `workflow_dispatch` lets PatchPilot decide when verification starts and pass inputs, such as which tests to run and which pull request the run belongs to. Push-triggered runs would start before the test selection is known and would run the full suite. Dispatch keeps the run targeted and keeps cost down across many repositories.

## Inputs and test selection

Test selection happens before dispatch. The runner passes the selection to the workflow as dispatch inputs. The workflow should do nothing clever with them: run what it was given and exit with a clear status. Keeping the logic on the PatchPilot side means one place to change when selection rules change.

## Reading results

After dispatching, the runner polls the workflow run until it finishes, then records the conclusion. Results are stored in SQLite so a later session or a retry can see what happened to each pull request without asking GitHub again. A run that never starts or never finishes is treated as a failure of verification, not as a pass.

## Failure handling

If the dispatch call is rejected, for example because the workflow file is missing on the target branch, the runner marks verification as failed for that pull request and keeps the reason. A missing workflow is a setup problem in the repository, not a test failure, and the stored reason should make that distinguishable. Retries should be limited and should not re-dispatch endlessly.

## Open points

Docker is used for packaging PatchPilot itself, and it is not yet settled whether the workflow should run tests in a container as well. Polling interval and timeout need tuning against real repositories. Check how the workflow behaves when two dispatches arrive for the same pull request close together.
