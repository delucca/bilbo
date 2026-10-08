---
id: 01KWEKJ7GFQTWDESZQFVK01A86
created: 2026-07-01T07:28-03:00
---

# mismatch-review-api gRPC endpoint

This note replaces the earlier note "mismatch review serves grpc". The new value: mismatch-review-api serves lark.review.v1.ReviewService on port 9444, and that replaces the earlier port. Anything that still points at the old port is stale.

## What this covers

Only the gRPC listener of mismatch-review-api in Ledgerlark: which service it exposes and where. It does not cover how mismatches are produced or how settlement files are ingested. Those live elsewhere.

## Current endpoint

mismatch-review-api exposes one gRPC service, lark.review.v1.ReviewService, on port 9444. Clients that talk to the review flow must dial that port. The old port no longer answers for this service, so a client left on it will fail to connect rather than get a different response.

## What changed

The port moved. The service name stayed the same, so protobuf definitions and generated Go stubs do not need regeneration just because of this change. Only the address that callers use is different.

## Who needs to update

Anyone who dials mismatch-review-api directly. Typical callers are the finance operations tooling that lists and resolves flagged mismatches, and any internal job that pushes review decisions back. Check their config for a hard-coded address and change it to use 9444.

## Infrastructure

The Terraform that defines the service, its load balancer or listener rules and any security group or firewall entries needs to allow 9444. If those still only allow the old port, traffic will be dropped before it reaches the process. Check the Terraform variables for the review service first, since the port is likely set in one place and reused.

## Health and readiness

Probes that target the gRPC port must use 9444 too. A readiness check left on the old port will mark healthy instances as down, and a rollout can stall or roll back for that reason alone.

## Kafka and the database

Nothing here touches Kafka topics or PostgreSQL. The review service reads and writes the same data as before. The port change is only about how clients reach it, so no migration and no topic changes are needed.

## Quick verification

Use a gRPC reflection or list call against host:9444 and confirm lark.review.v1.ReviewService shows up. If the call is refused, the instance is probably running an older build or the network rules were not updated.

## Gotchas

Docs, runbooks and local dev configs may still mention the old port. Treat the earlier note as superseded and trust this one. Mixed environments are possible during a rollout, with some instances on the old port and some on 9444, so do not conclude a client is broken from a single failed call.

## Open items

Confirm that every environment has moved to 9444, not only the one tested. Remove leftover references to the old port once nothing uses it.

## Related notes

The earlier note "mismatch review serves grpc" should be treated as replaced by this one. Keep this note as the single place for the review service address.
