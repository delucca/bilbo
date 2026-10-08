---
id: 01JTFCJ3KCPH24QF4YAPS1XS0P
created: 2025-05-05T01:51-03:00
sources:
  - "code: src/dispatcher/server.ts"
---

# ci-dispatcher design

The ci-dispatcher is the part of PatchPilot that listens for GitHub webhooks and turns them into work. It receives GitHub webhooks on port 8787 at the path /hooks/github. Everything else in this note is about what happens around that one entry point: how a delivery is accepted, how it is recorded, how it becomes a run of targeted tests, and how the result gets back to the pull request. This is a design note, so it says what the ci-dispatcher is meant to do and why. Where something is still open, it says so.

PatchPilot opens dependency upgrade pull requests across many repositories and then checks that each upgrade is safe by running only the tests that matter for the change. Platform engineers who maintain many repositories use it. They do not want to watch each pull request by hand, so the ci-dispatcher is the piece that reacts to GitHub events on their behalf.

## Purpose and scope

The ci-dispatcher has one job: take events from GitHub, decide whether they concern a PatchPilot pull request, and start or update the verification work for that pull request. It does not decide which dependency to upgrade. That belongs to the upgrade planner. It does not pick the tests either. It asks the test selection step, which returns a targeted set for the changed dependency, and then it launches that set through GitHub Actions.

What the ci-dispatcher owns:

- The HTTP listener for webhooks, on port 8787, path /hooks/github.
- Verifying that a delivery really came from GitHub.
- Recording every accepted delivery in SQLite so nothing is lost on restart.
- Deciding which deliveries matter and dropping the rest.
- Starting verification runs and tracking their state.
- Reporting the outcome back on the pull request.

What it does not own:

- Creating the upgrade pull requests.
- Choosing which tests are relevant. It consumes that answer.
- Running the tests themselves. GitHub Actions does that, inside Docker where the repository needs it.
- Merging. A human or a separate merge policy decides that.

Keeping the scope this small is on purpose. An earlier idea was to let the listener also create pull requests when a new version appeared. That mixed two failure modes, an inbound event flood and an outbound API budget, in one process. The ci-dispatcher stays reactive.

## The webhook endpoint

The listener binds port 8787 and serves one route, /hooks/github. Every GitHub App or repository webhook that PatchPilot cares about is pointed at that path. A sample of the shape a configured webhook target takes, using only the values that are fixed:

```
Payload URL: http://<ci-dispatcher host>:8787/hooks/github
Content type: application/json
```

The route accepts POST only. Any other method gets a plain rejection, and any other path gets a not-found response. There is no health route on the same path. If a liveness check is needed, it should be a separate route, and that has not been added yet. For now the process supervisor treats a successful bind of the port as alive.

The handler does as little as possible while the request is open. It reads the raw body, checks the signature, writes a row, and answers. It does not call GitHub, does not start any run, and does not touch Docker before answering. GitHub expects a quick reply and will mark a delivery as failed if the receiver is slow, so all real work happens after the response has gone out.

The raw body matters. Signature verification has to run over the exact bytes GitHub sent. If a JSON body parser runs first and the body is re-serialized, the signature will not match. The listener therefore captures the raw bytes before any parsing, and parses from those same bytes after verification passes.

## Request verification

Each webhook is configured with a shared secret. The ci-dispatcher computes an HMAC of the raw body using that secret and compares it to the signature GitHub supplies in the delivery headers. The comparison is constant-time. A missing signature, a malformed one, or a mismatch all produce the same rejection, with no detail in the response about which case it was. The log line on the server side does say which case, so an operator can tell a misconfigured secret from a probe.

The secret comes from configuration, not from the repository. It is read at start-up. Rotating it means restarting the process, or, if multiple secrets are accepted during a rotation window, adding the new one before changing it on the GitHub side. The design allows a short list of accepted secrets for exactly that reason. The old one is removed after GitHub is switched over.

Verification happens before any parsing of the payload as JSON and before any database write. An unverified request never reaches SQLite. That keeps random traffic on the port from filling the store.

Open question: whether to also restrict by source address. GitHub publishes its hook ranges, but they change, and an allow-list that goes stale will silently drop real deliveries. The current decision is signature only. If the endpoint ends up exposed publicly, this should be revisited with a reverse proxy in front.

## Event intake and the delivery log

After verification, the handler stores the delivery in SQLite and answers with success. The stored record holds the delivery identifier GitHub assigns, the event type, the repository, the raw payload, the time received, and a processing state. The delivery identifier is unique in the table. A redelivery of the same identifier, which GitHub does when asked or after a failure, hits the unique constraint and is acknowledged without creating a second record.

The processing state starts as pending. A worker loop in the same process picks up pending rows, handles them, and marks them done, ignored, or failed. Because the row is written before the reply, a crash after the reply cannot lose a delivery. On start-up the worker first looks for rows left in an in-progress state from a previous run and returns them to pending. Handlers are written to be safe to run twice for that reason.

Why SQLite and not a queue service: the volume is modest, the process is a single instance, and the platform team did not want another piece of infrastructure to run. SQLite in write-ahead mode handles the write pattern fine, which is many small inserts and a single reader loop. If the ci-dispatcher is ever run as several instances, this choice must be reopened, since a shared file is not a good coordination point across hosts.

Raw payloads are kept for a limited time and then pruned, so the file does not grow without bound. The retention period is a configuration value. The state rows stay longer than the payloads, because they are small and useful when answering why a pull request was or was not verified.

## Event filtering and routing

Most deliveries are of no interest. The worker looks at the event type first and drops anything it does not handle, marking the row ignored. The events that matter are the ones that change the state of a PatchPilot pull request or report on a check run attached to it:

- A pull request being opened, updated with new commits, reopened, or closed.
- A check run or workflow run finishing.
- A comment or label action that asks for a re-run, where that feature is enabled.

For pull request events, the next filter is whether the pull request belongs to PatchPilot. The ci-dispatcher recognizes its own pull requests by the branch naming and by a marker PatchPilot places in the pull request description when it opens one. Either alone is not enough. A human could copy a branch name, and a description can be edited. Requiring both avoids starting runs for pull requests PatchPilot did not create. This is a heuristic and the design accepts that. The cost of a false negative is a missing verification that a human can request again. The cost of a false positive is wasted compute.

Routing after that is by event type into a small set of handlers. Each handler is a plain function that takes the stored delivery and a context holding the database handle and a GitHub client. There is no plugin system. Adding an event means adding a function and one line in the routing table.

## Dispatching verification runs

When a PatchPilot pull request is opened or gets new commits, the handler asks the test selection step which tests to run for the changed dependency. The answer is a description of a targeted set, not a full suite. The ci-dispatcher then starts a GitHub Actions workflow in the target repository and passes that description as input. The workflow runs the selected tests, using Docker where the repository defines a container for its test environment, and reports back through the normal check run mechanism.

The ci-dispatcher records one run row per dispatch, tied to the pull request and the commit it was started for. The row holds the state, the time started, and a reference to the workflow run once GitHub reports one. Dispatching a workflow does not return the run identifier directly, so the ci-dispatcher matches the new run to its row by commit and by a correlation value it put into the workflow inputs. This is the least pleasant part of the design. If matching fails, the run is marked lost after a timeout and retried once.

When new commits arrive while a run is in progress for the same pull request, the older run is superseded. The ci-dispatcher asks GitHub to cancel it and marks the row accordingly, so results for stale commits are never posted as if they were current. Only the latest commit's run can set the final status.

The test selection step may fail or return nothing useful, for example when the dependency is used everywhere. In that case the handler falls back to a broader set and notes the fallback in the run row, so the pull request comment can say that the run was wide and why.

## Reporting results

When a workflow run finishes, GitHub sends another delivery to the same endpoint, on port 8787 at /hooks/github. The worker matches it to a run row, reads the conclusion, and updates the row. It then updates the pull request: a commit status or check summary, and a short comment on the first result for that commit. Later results for the same commit edit the existing comment instead of adding new ones, to keep the pull request readable.

The comment says which dependency changed, which tests were selected and why in a sentence, the outcome, and a link to the workflow run. Failures include the first failing test name where the workflow output makes it available. The ci-dispatcher does not paste logs. Logs stay on GitHub, and the link is enough.

The outcome is one of passed, failed, or inconclusive. Inconclusive covers a lost run, a cancelled run that was not superseded by the ci-dispatcher, and a workflow that errored before any test ran. Treating those as failures would make platform engineers distrust the signal, so they are separate and the comment says so.

## Failure handling and retries

Three kinds of failure matter here: inbound, outbound, and internal.

Inbound failures are bad signatures and malformed bodies. They are rejected quickly, logged, and never stored. Repeated rejections from one source are only logged. There is no blocking logic yet.

Outbound failures are calls to the GitHub API that fail or are rate limited. The worker retries these with a growing delay and a cap on attempts. When a rate limit response includes a reset time, the worker waits for it instead of guessing. A delivery that keeps failing is marked failed with the last error stored on the row. Failed rows are kept and can be put back to pending by an operator. The worker does not retry failed rows by itself forever, since an endless loop against a revoked token helps nobody.

Internal failures are bugs in handlers. An exception in one handler marks that delivery failed and the loop moves on. One bad payload must not stall the queue. The error and a stack trace go into the log, and the row records a short message.

Because handlers can run twice after a crash, each one checks the current state before acting. A handler that starts a run first looks for an existing run row for the same pull request and commit. A handler that posts a comment looks for its own earlier comment and edits it. This idempotence is the main reason the design stores state in SQLite at all.

## Deployment and operations

The ci-dispatcher is a Node.js service written in TypeScript. It ships as a Docker image and runs as a single container with the SQLite file on a mounted volume. Losing the volume loses the delivery log and run state. GitHub can redeliver recent webhooks, but runs that were in flight would have to be rediscovered, so the volume should be backed up like any other state.

Configuration comes from environment variables: the webhook secret or secrets, the GitHub credentials, the database location, and the retention settings. The port is 8787 by default and the path is fixed at /hooks/github. Changing the port in a deployment is possible, but then the webhook URL on the GitHub side has to change with it, and that is easy to forget. If deliveries stop arriving after a redeploy, check the port mapping and the webhook's recent deliveries page on GitHub before looking at the code.

Logs are structured, one line per event, with the delivery identifier on every line that relates to a delivery. That identifier is the thing to search for when someone asks why a pull request did not get verified. The row in SQLite then shows whether the delivery was ignored, failed, or processed, and the run row shows what happened next.

Graceful shutdown stops accepting new connections, lets the current request finish, lets the worker complete the delivery it is holding, and then exits. Anything not finished stays pending and is picked up on the next start.

## Testing approach

Handlers are tested with recorded payloads from real GitHub deliveries, with secrets and private names scrubbed. Each test stores a delivery in an in-memory SQLite database, runs the handler against a fake GitHub client, and checks the resulting rows and the calls the fake received. This covers routing, filtering, and idempotence without network access.

The endpoint itself is tested with a signed request built in the test using a known secret: a correct signature is accepted and stored, a wrong one is rejected and nothing is stored, and a repeated delivery identifier is acknowledged without a second row. One test posts the body re-serialized with different whitespace, to make sure the raw-body rule is actually enforced.

There is also a small end-to-end check that runs in GitHub Actions against a throwaway repository, using a real webhook through a tunnel. It is slow and flaky by nature, so it is not part of the normal pull request gate. It is run before releases and when the dispatch matching logic changes.

## Open questions and later work

Several things are undecided and should not be assumed from the code as it stands.

- Multiple instances. The single-process design with a local SQLite file will not scale sideways. If the volume of webhooks grows enough to need that, the store and the worker coordination both change.
- A health route. A separate route for liveness and readiness would help the supervisor, and it must not share the webhook path.
- Source address restrictions. See the verification section. Signature checking is the only gate today.
- Matching dispatched runs. The correlation approach works but is indirect. If GitHub offers a way to get the run identifier at dispatch time, switch to it and delete the timeout and retry logic.
- Re-run requests by comment. This exists behind a switch, and who is allowed to ask for a re-run is not settled. For now only users with write access count, and that check needs a second look.
- Flaky test handling. A selected test that fails intermittently produces a failed outcome that a human then discounts. The ci-dispatcher could retry once and report both results. This was discussed and not built.

When changing anything here, keep the order of the handler path: read the raw body, verify, store, reply, then work. Every problem we expect from this service comes from breaking that order.
