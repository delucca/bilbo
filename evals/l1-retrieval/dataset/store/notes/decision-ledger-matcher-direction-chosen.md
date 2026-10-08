---
id: 01JX6X5C86NCBAJSQGZ5X8KXW9
created: 2025-06-08T01:35-03:00
---

# ledger-matcher: general direction

We settled on a general direction for ledger-matcher and this is the short version so nobody has to rebuild the argument later. It is a direction, not a spec. Details live in code and in the related note [[ledger-entries-review-themes]].

## Scope

ledger-matcher takes parsed settlement records from the card processor and compares them with internal ledger entries. Anything that does not line up goes to review. It does not fix data itself.

## Core choice

Keep matching deterministic and explainable. A reviewer should be able to see why a pair matched or why it did not. We preferred this over a clever scoring approach that is hard to defend to finance people.

## Language and runtime

Go, as a plain service. Small packages, few dependencies, boring concurrency. Matching logic stays pure so it can be tested without a database or a broker.

## Input path

Settlement records arrive through Apache Kafka. The matcher consumes them and treats redelivery as normal. Handling the same record twice must give the same result.

## Storage

PostgreSQL is the source of truth for ledger entries, match results and review flags. We lean on constraints in the database to keep duplicates out rather than trusting application checks alone.

## Matching approach

Match in passes, strictest first, then looser ones. Looser passes only run on what the strict pass left over. Every result records which pass produced it.

## Mismatch handling

A mismatch is data, not an error. It gets a reason category and goes to the review queue. We do not drop or auto-resolve unclear cases.

## Interfaces

Other services talk to ledger-matcher over gRPC. Keep the contract small and additive, no breaking changes without a migration plan.

## Infrastructure

Terraform describes the deployment. Nothing for ledger-matcher should be created by hand in an environment.

## Idempotency

Every write keyed so replays are safe. This matters more than throughput for now.

## Observability

Log each decision with enough context to replay it. Count outcomes by category so drift shows up early.

## Testing

Table tests for matching rules, using realistic settlement samples with sensitive data removed. Integration tests against a real PostgreSQL, not a mock.

## Not decided

Tuning for volume, retention of old match results, and how review feedback loops back into the rules. Open for later.

## Revisit when

Reviewers keep asking why something did not match, or the passes start to overlap in confusing ways.

## Example shape

```text
settlement record -> ledger-matcher -> matched | review
```

## Pointers

Read [[ledger-entries-review-themes]] for what reviewers tend to flag, and check it before adding a new mismatch category.
