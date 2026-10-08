---
id: 01KVZPMJ30GB58B0XD2N940RXR
created: 2026-06-25T12:33-03:00
sources:
  - "code: src/auditmesh/evaluator/handler.py"
---

# opa-eval-lambda design

opa-eval-lambda is the part of AuditMesh that decides whether a single cloud resource breaks a policy. It takes one resource document at a time, runs it through Open Policy Agent, and returns the violations it found. It does not talk to Jira and does not decide what a ticket looks like. It answers one question: does this document violate any rule, and which ones. This note records how it is built and why, so the next person does not have to rebuild the picture from the code.

The component used to be called `regorunner`. It is called `opa-eval-lambda` now. Old dashboards, old log group names, old commit messages and some chat history still say `regorunner`. If you meet that name, it is this component. Do not create anything new under the old name, and when you touch an old reference, move it to `opa-eval-lambda`.

It runs on `Python 3.12` as an AWS Lambda function. For each resource document it calls the OPA binary shipped in a Lambda layer at `/opt/bin/opa`. The function does not embed OPA as a library and does not run an OPA server. It starts the binary as a child process, hands it the document, reads the result, and exits the child. That choice shapes most of what follows.

```
Python 3.12 handler
  -> child process: /opt/bin/opa
     input: one resource document
     output: violations for that document
```

## Where it sits in AuditMesh

AuditMesh scans cloud infrastructure configs and produces remediation tickets for cloud security teams. The flow has a few stages. A collector gathers configuration from the accounts being scanned and turns it into resource documents. Those documents are queued. opa-eval-lambda picks them up, evaluates them against the current policy bundle, and writes findings to DynamoDB. A later stage reads the findings, groups them, and opens or updates Jira tickets.

opa-eval-lambda is the only stage that knows how to run Rego. The collector knows about cloud APIs and the ticketing stage knows about Jira. Keeping Rego knowledge in one place means a policy author can test a change against sample documents without touching the rest of the system.

The contract with its neighbours is small on purpose. In: a resource document plus enough metadata to say which account, which region and which resource type it came from. Out: a list of findings, each with a rule identifier, a short message, and a pointer back to the resource. Everything else is internal. If a neighbour needs more from this component, add it to the contract deliberately instead of letting it leak through the findings payload.

## Why a binary in a layer

The main alternatives were an OPA server running as a separate service, a Rego-to-WASM compile step loaded inside the Python process, or a Python re-implementation of the rules. We picked the binary in a layer.

A separate server would add a network hop, something to keep alive, and a second deployment to coordinate with the function. For a workload that is bursty and mostly idle between scans, an always-on service costs more than it earns. A Lambda that starts, evaluates, and goes away fits the scan pattern better.

WASM was attractive but the policy authors write full Rego and use built-ins that are awkward or unsupported in the compiled form. We did not want policy authors to learn which parts of the language are safe. The binary supports what they already write and test locally with the same tool.

A Python re-implementation was rejected quickly. Two sources of truth for policy meaning is the failure mode this whole component exists to avoid.

The layer approach has costs. The binary version is tied to the layer version, so upgrading OPA means publishing a new layer and pointing the function at it. Process start has overhead for every evaluation. We accept both. The first is easy to see and easy to roll back. The second is handled by how we batch work, described below.

## Evaluation flow

The handler does the same steps for each incoming message.

First it validates the envelope. A message without a usable resource document or without the metadata we need is rejected early with a clear error, and it never reaches OPA. This keeps garbage from being reported as a policy result.

Second it loads the policy bundle. The bundle is read once per cold start and kept in memory or in the temporary directory of the execution environment, so warm invocations reuse it. When a new bundle version is published, new execution environments pick it up as they start. Old ones keep the bundle they had until they are recycled. For a short period two bundle versions can be live at once. Findings record which bundle version produced them, so a reader can tell the difference.

Third it runs `/opt/bin/opa` as a child process against the document. The handler passes the document as input, asks for the violation set, and reads the result back as structured data. The child is given a time limit well below the function timeout, so a runaway policy cannot eat the whole invocation and leave nothing to report.

Fourth it maps the OPA output into findings. This is a plain translation step in Python. It does not add policy logic. If a mapping needs an if-statement that decides whether something is a violation, that logic belongs in Rego.

Fifth it writes findings to DynamoDB and acknowledges the message. Writes are keyed so that evaluating the same document twice with the same bundle version produces the same item, not a duplicate. That makes redelivery from the queue harmless.

## Failure handling

There are three kinds of failure and they are treated differently.

Bad input is a permanent failure. The message goes to the dead-letter path with the reason attached. Retrying does not help.

A policy error is also mostly permanent. If OPA exits with an error for a given document, the usual causes are a Rego runtime error on an unexpected shape in the document, or a policy bug. We record the failure against the resource and the rule set, not as a clean result. A resource that could not be evaluated must never look like a resource with no violations. This is the rule that matters most in this component: silence must not mean compliant. The downstream stage treats an evaluation failure as its own state and surfaces it to the security team.

Infrastructure trouble is transient. A DynamoDB throttle, a timeout talking to a dependency, or a killed child process is retried through the queue's normal redelivery. Because writes are idempotent, redelivery is safe.

The handler logs enough to tell these apart without reading the document itself: the resource type, the bundle version, the outcome class, and the OPA exit status when there was one. Resource contents are not logged. They can contain secrets that a cloud config happens to hold, and logs are read by more people than the findings table is.

## Performance and limits

Starting a process per document is the obvious cost, so the handler processes the messages it receives in a batch within one invocation instead of one invocation per resource. Within a batch, documents are evaluated one after another. If profiling later shows the process start dominates, the next step to try is passing several documents to one OPA run, not switching to a server.

Memory matters more than CPU for the large documents, since some resource types, such as big IAM policy sets, produce sizeable inputs. The function memory setting is chosen with those in mind. Lambda gives CPU in proportion to memory, so a higher setting also speeds up evaluation, and we treat the two as one knob.

Cold starts pay for unpacking the layer and loading the bundle. Provisioned concurrency has been discussed but not adopted, since scans are scheduled and a few slow first invocations do not hurt anyone. Revisit this if scans become near real time.

The layer size is bounded by Lambda limits, which is another reason to keep the layer to the OPA binary and nothing else. Python dependencies go with the function package, not the layer, so upgrading one does not force the other.

## Testing and local work

Policy authors test Rego with the same OPA tooling they use everywhere else, against sample documents kept next to the policies. That layer of tests does not need this component at all.

For the component itself, tests cover the envelope validation, the output mapping, the failure classification, and the idempotent write key. The OPA call is behind a thin wrapper so tests can replace it with canned output. A smaller set of integration tests runs the real binary against a few fixed documents and checks that known violations come back. Those need the binary present, so they run in an environment that has it, either the layer contents unpacked locally or a container built from the same layer.

When debugging a production finding, the quickest path is to take the recorded bundle version and the resource document, run the binary locally with that bundle, and compare. Because findings store the bundle version, this works even after the policies have moved on.

## Operational notes

- Upgrading OPA: publish a new layer, point a test alias of the function at it, run the integration documents, then shift the main alias. Keep the previous layer version available so rollback is a pointer change.
- Changing policies: publish a new bundle. Expect a short overlap where both versions are live. Do not assume every finding in a scan came from the same bundle.
- Old name: search logs, alarms and dashboards for `regorunner` when something seems to be missing. Some of them may still be attached to the old name and quietly show nothing for `opa-eval-lambda`.
- Alarms worth keeping: rate of evaluation failures per resource type, dead-letter depth, and function errors. A jump in evaluation failures for one resource type usually means the collector changed the document shape, not that the policy broke.
- Security: the function role should read the bundle and write findings, and nothing more. It has no reason to call back into the cloud accounts being scanned.

## Open questions

Whether to move to multi-document OPA runs depends on measurements we have not taken under a realistic load. Whether to cache evaluation results by document hash and bundle version is also open. It would save work on unchanged resources between scans, but it adds a cache whose invalidation rules must follow bundle changes exactly, and a stale entry would hide a real violation. Given the rule that silence must not mean compliant, we have leaned against it until the cost of re-evaluating everything actually hurts.

The last open item is how to expose per-rule timing from the child process without logging document content. Right now we only see total time per document. If a slow policy shows up, we will want a per-rule view, and that needs a safe way to produce it.
