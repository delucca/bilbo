---
id: 01M2HBRARGKRACM6XY5MDMTATV
created: 2026-09-14T22:43-03:00
sources:
  - "code: internal/audit/sink.go"
---

# keyvane-audit-log design

keyvane-audit-log is the component that records what Keyvane did with credentials: who asked, what was issued, and for how long. It writes one JSON event per line to `/var/log/keyvane/audit.jsonl`. Each event carries the fields spiffe_id, ttl_seconds and `request_id`. This note is the short design record, written fast, so some parts are left general on purpose.

## Purpose

Security engineers need a trail they can read after the fact. Application teams need to see why a credential was issued or refused. keyvane-audit-log gives both a single plain file format that ordinary tools can read.

## Output location

The file is `/var/log/keyvane/audit.jsonl`. keyvane-audit-log only appends to it. Nothing else in Keyvane should write there. If the path is not writable the component should fail loudly rather than drop events quietly.

## Line format

One JSON object per line, no pretty printing, no multi-line events. This keeps the file safe to tail, grep and ship with line-based collectors. A reader that sees a partial last line should skip it and read it again later.

## Fields

Three fields are the stable core of every event.

### spiffe_id

The SPIFFE identity of the caller that made the request. It comes from the verified mTLS peer certificate, never from the request body. This is what ties an event to a real workload.

### ttl_seconds

The lifetime of the issued credential, in seconds. For events that did not issue anything, the value describes what was asked for or is left at zero, depending on the event type. Check the code before relying on that edge.

### request_id

A unique id for the request. The same `request_id` should show up in the other Keyvane components that touched the request, so an engineer can follow one request across services. It is the main join key when investigating.

## Where events come from

The credential issuing path and the rotation path both emit events. Reads of configuration and health checks do not. Events are emitted after the decision is made, so they describe the outcome and not the attempt.

## Relationship to Vault

HashiCorp Vault has its own audit devices. keyvane-audit-log does not replace them. Vault records what happened inside Vault; keyvane-audit-log records what Keyvane decided and handed out. When debugging, compare both using the request id where it is available.

## Relationship to etcd

Coordination state lives in etcd, but audit events are not stored there. Keeping the log as a local file avoids making the audit trail depend on the cluster being healthy. The cost is that each node has its own file, and collection has to be handled outside the component.

## Identity and transport

Callers are authenticated with mTLS and identified through SPIFFE. The audit event reuses that identity as is. If the identity cannot be verified, the request is rejected and the rejection is still logged where possible.

## Secrets must not appear

Credential values, tokens and private keys never go into an event. Only identifiers and metadata are logged. Any new field should be reviewed against this rule before it is added.

## Ordering and timing

Lines are written in the order the component receives events. Across nodes there is no global order. Do not assume clock agreement between nodes when merging files.

## Write behavior

Writes are append-only and one event per write call, so lines do not interleave from concurrent goroutines. A mutex or a single writer goroutine serializes access. The choice between those is an implementation detail and can change.

## Rotation of the file

The log file itself needs rotation by an external tool, since keyvane-audit-log does not manage retention. After rotation the component must reopen the file so it does not keep writing to the old one. Do not confuse this with secret rotation, which is a different feature.

## Failure modes

Disk full, missing directory and wrong permissions are the usual causes of trouble. In each case the component reports an error and the issuing path should decide whether to refuse the request. The preferred stance is fail closed: no audit record, no credential.

## Compatibility

Consumers should ignore unknown fields. New fields may be added over time; the three core fields should not be renamed or removed without a clear migration.

## Open questions

- How to ship files from every node to one place.
- Whether failed authentication attempts need extra fields.
- Whether a signed or hash-chained format is worth the cost.

## Pointers

Start from the Go package for keyvane-audit-log in the repo and look for where events are built. The field names above are the quickest thing to grep for when tracing a bug.
