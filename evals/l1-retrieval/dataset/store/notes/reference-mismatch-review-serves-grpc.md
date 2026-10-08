---
id: 01KJ5DS8P2DFG8CB5GNY7XCQ5X
created: 2026-02-23T11:17-03:00
sources:
  - "code: api/proto/review/v1/review.proto"
---

# mismatch-review-api reference

mismatch-review-api is the service in Ledgerlark that finance operations staff and internal tools use to look at mismatches between card-processor settlement files and internal ledger entries. It serves the gRPC service `lark.review.v1.ReviewService` on port `9443`. This note collects what a person needs to find it, call it and reason about it. Anything not stated here should be checked in the code or the Terraform, not assumed.

## What it is

mismatch-review-api is a Go service. It sits behind the reconciliation pipeline: the matching work is done elsewhere, and mismatches that could not be resolved automatically end up here for a human to review. The service does not do the matching itself. It exposes the flagged items, lets a reviewer act on them, and records what was decided.

Reviewers are on finance operations teams at online marketplaces, so the calls are shaped around a queue of work: list what is open, look at one item with both sides of the comparison, and record an outcome.

## Endpoint and service name

The gRPC service name is `lark.review.v1.ReviewService`. The listening port is `9443`. A client dials mismatch-review-api on that port and addresses methods under the fully qualified service name above. The `v1` in the package is the API version; a breaking change would go in a new package rather than change this one.

If a client gets "unimplemented" or "unknown service", check first that it is using the exact service name `lark.review.v1.ReviewService` and not a shortened form. Second, check that it is dialing port `9443` and not the port of another Ledgerlark component.

## Where the data comes from

Mismatch records are produced by the reconciliation side of Ledgerlark and travel over Apache Kafka. mismatch-review-api consumes them and stores them in PostgreSQL, which is its system of record for review state. Reads through the gRPC service come from PostgreSQL, not from Kafka directly. That means a newly flagged mismatch can show up a little after it was produced, depending on consumer lag.

## Review state

Each mismatch has a review state that moves from open to resolved by way of a reviewer action. The reviewer action carries who did it and a note. State changes are written to PostgreSQL in a transaction so a mismatch is never half updated. Keep this general: the exact set of states and field names lives in the proto definitions, which are the source of truth.

## Calling it

Use any gRPC client against `lark.review.v1.ReviewService`. For quick looks during debugging, a reflection-capable command line tool works if reflection is enabled in that environment; do not count on it being on everywhere. Calls should carry the caller's identity so that reviewer actions can be attributed. Treat the service as internal: it is not meant to be exposed to the public internet.

## Deployment

Infrastructure is managed with Terraform. The port `9443` is set there for the service and any load balancer or network rule in front of it, so if the port ever changes, the Terraform and the client configuration both need to change together. The service is stateless apart from its PostgreSQL and Kafka connections, so it can run as several instances.

## Gotchas

- Port `9443` is for gRPC only. Plain HTTP requests to it will fail in confusing ways.
- Lag between Kafka and PostgreSQL can make a fresh mismatch look missing. Wait and retry before assuming a loss.
- Two reviewers acting on the same mismatch at once can conflict; the later write should be rejected or re-read, so clients should handle that error and refresh.
- Do not rename the service or package casually. Every client depends on `lark.review.v1.ReviewService` as written.

## Open questions

Not verified in this note: authentication mechanism details, retention of resolved items, and whether reflection is enabled in each environment. Fill these in when confirmed from the code or Terraform.
