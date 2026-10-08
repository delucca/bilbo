---
id: 01K6FNXCH9CZN23M3BRZ000TVB
created: 2025-10-01T07:43-03:00
---

# mismatch-review-api: next steps

This is a rough plan for where mismatch-review-api goes next. It is written in general terms on purpose. Nothing here fixes a value, a schedule or a threshold; those get settled when each piece is picked up. The order below is roughly the order I would work in, but the early items can overlap.

mismatch-review-api is the service finance operations people and their tooling use to look at flagged mismatches between processor settlement files and internal ledger entries, and to record what they decide. The matching itself happens upstream. This service sits on the review side: it reads flagged items, lets a reviewer act on them, and publishes the outcome so other parts of Ledgerlark can react.

## Where things stand

The service exposes a gRPC surface for listing, fetching and resolving mismatches, backed by PostgreSQL. Resolution outcomes go out on Kafka. It works for the normal path. The rough edges are around volume, concurrent reviewers, unclear error behavior, and how much of the review history can be reconstructed afterwards. The next steps are mostly about those edges rather than new features.

## Tighten the API contract

Go through the protobuf definitions and decide what is really part of the contract and what leaked in from internal models. Remove or mark anything that clients should not depend on. Make field naming consistent across the list, get and resolve calls. Write down which fields are optional and what absence means.

Pay attention to compatibility rules for the proto files. Additive changes only unless there is a coordinated release with every client. Add a check in CI that flags breaking changes to the schema so nobody finds out in production.

## Error handling and status codes

Audit which gRPC status codes the handlers return today. Several failure cases probably collapse into a generic internal error. Separate at least these: bad input, item not found, item already resolved by someone else, caller not allowed, and dependency unavailable. Give clients enough detail to act without exposing database internals.

Decide which errors are safe to retry and say so in the docs for each call. Make the resolve call safe to retry from the client side, so a timeout followed by a second attempt does not produce a double resolution or a duplicate event.

## Concurrency between reviewers

Two reviewers opening the same mismatch is normal. The service needs a clear story for it. Options to weigh: optimistic version checks on the row, a soft claim that expires, or both. My leaning is optimistic checks first because they are simple and need no cleanup job, then a claim feature only if reviewers ask for it. Whatever is chosen, the loser of a race must get a clear, specific error rather than a silent overwrite.

Test this with real concurrent calls against a real PostgreSQL instance, not only mocks.

## Pagination and filtering

List calls need stable, cursor-based pagination so that results do not skip or repeat while new mismatches arrive. Review which filters reviewers actually use: processor, status, age, amount range, assignee. Add the ones that are missing and make sure each has index support. Look at query plans on a database with production-like volume before calling it done.

Sorting should be deterministic, with a tiebreaker, so that two pages never disagree.

## Database schema and migrations

Review the tables behind the review workflow. Check that status values are constrained at the database level and not just in Go code. Check that foreign keys and indexes match the real query patterns. Plan any change as a safe sequence: add, backfill, switch reads, then remove, so the service can roll back at each step.

Keep migrations reviewable and small. Decide how migrations are run during deploys and who is responsible when one is slow.

## Kafka publishing

Resolution events should be published reliably relative to the database write. The weak point is the gap between committing the row and producing the message. Look at moving to a transactional outbox: write the event into an outbox table in the same transaction, and have a publisher drain it. That removes the lost-event and phantom-event cases.

Consumers need to tolerate duplicates, so each event should carry a stable key they can dedupe on. Document the event schema separately from the gRPC schema and apply the same compatibility rules. Check partitioning so that events for one mismatch stay in order.

## Audit trail

Finance teams will want to know who resolved what, when, and with which stated reason. Make sure every state change writes an immutable history record with the actor, the previous and new state, and any free-text note. Reviewers should be able to fetch that history through the API.

Do not allow edits or deletes of history rows from the service code. Think about retention with the finance side before adding anything that purges data.

## Authentication and authorization

Confirm how callers are identified on the gRPC side and how identity reaches the handler. Define roles in plain terms: read-only, reviewer, and an administrative role for reassignment or reopening. Enforce them in one interceptor rather than inside each handler, so a new method cannot forget the check.

Make sure the actor recorded in the audit trail comes from the verified identity and never from a field the client supplies.

## Observability

Add or fix the basics: request counts and latency per method, error counts by status code, outbox lag, and queue depth of unresolved mismatches. Logs should carry a request identifier and the mismatch identifier, and should not carry card data or full account details. Check logging for that on every handler that touches settlement data.

Write down what an on-call person should look at first when review calls get slow or events stop flowing.

## Infrastructure with Terraform

Review how the service is described in Terraform: compute, database access, Kafka topics and their access rules, and secrets. Anything created by hand in a console should be brought into code or removed. Keep environments structurally the same so a change tested in staging means something.

Topic configuration changes and database changes should be reviewed together with the service change that needs them, not afterwards.

## Testing and rollout

Build up a small set of integration tests that exercise the main review flow end to end: list, fetch, resolve, event emitted, history recorded. Add tests for the race and retry cases above. Run them in CI against disposable PostgreSQL and Kafka instances.

Roll out changes behind careful sequencing: schema first, then the service that can use it, then the clients. Prefer small releases. Keep a rollback path for each, and note in the change description what to watch after deploy.

## Open questions

- Do reviewers need a claim or assignment feature, or is conflict detection enough?
- How long must review history be kept, and who owns that call?
- Which clients call this service today, and can they all handle the stricter error codes?
- Is the outbox worth running inside this service, or should a shared component own it?
- What does finance operations want to see first when they open the queue: oldest, largest, or by processor?

Answers to these should be recorded as separate decision notes when they are settled, not edited into this plan.
