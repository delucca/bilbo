---
id: 01KTYXT9BPHFJ9DB6GRVVFKMW8
created: 2026-06-12T19:04-03:00
---

# mismatch-review-api design

mismatch-review-api is the service finance operations reviewers use to look at the mismatches Ledgerlark finds between card-processor settlement files and internal ledger entries. This note records how the read side is shaped and why. It is written from the design as it stands, not from a fresh code read, so check the proto definitions before relying on small details.

## Purpose and boundaries

mismatch-review-api only serves review. It does not do the matching. Reconciliation workers in Go compare settlement records with ledger entries and write mismatches into PostgreSQL. mismatch-review-api reads from there and lets a reviewer act on what it finds. Keeping matching and review apart means a slow reviewer session or a large export cannot hold up reconciliation runs, and a reconciliation backlog does not freeze the review screens.

The service speaks gRPC to its callers. The web console for the finance team goes through a gateway that talks gRPC to it. Internal tools such as scripts that sweep old mismatches use the same calls. There is no second REST surface to keep in sync.

## ListMismatches

The `ListMismatches` call of mismatch-review-api is a server-streaming RPC. It returns pages of `50 mismatches` each. The client sends one request with its filters, and the server sends back a stream of page messages, each holding up to 50 mismatches, until the result set is finished or the client cancels.

Why streaming and not classic page tokens:
- Reviewers often work through a long queue in order. One open stream avoids a round trip per page and avoids re-running the filter query for every page.
- The server can read from a single PostgreSQL cursor or keyset position and push pages as it reads them, so memory stays flat even for big queues.
- Cancelling the stream gives a clean way to stop work on the database side when the reviewer closes the screen.

Things to keep in mind:
- The last page can hold fewer than 50 mismatches. A short page is not an error and does not by itself mean there is more or less data; the end of the stream is the signal that it is done.
- Ordering has to be stable, otherwise a reviewer could see a mismatch twice or miss one when rows change during a long stream. The query orders on a fixed key and resumes from the last key it sent, not by offset.
- Backpressure matters. If the client reads slowly, gRPC flow control slows the server, and the server must not buffer pages without a limit.
- Clients that need to resume after a dropped connection should restart with the last mismatch they saw as a starting point in the filter, not replay from the start.

## Data and consistency

Mismatches live in PostgreSQL. The stream reads a consistent view for the duration of one stream where possible, so a reviewer's list does not shift underneath them. Changes made by other reviewers or by reconciliation during the stream may not show until the next call. This is accepted. The review screen refreshes by opening a new stream.

Status changes made by reviewers, such as marking a mismatch as explained or escalated, are separate unary calls, not part of the stream. Those writes go to PostgreSQL and then produce an event on Apache Kafka so downstream consumers, like reporting and the ledger adjustment flow, hear about the decision. The write and the event must not drift apart, so the event is published from a stored outbox row rather than directly in the request path.

## Operations and open points

Deployment and network setup for the service are managed with Terraform alongside the other Ledgerlark services. Streams are long-lived, so load balancer idle timeouts and deploy-time connection draining need care; a rolling deploy will cut streams and clients must be ready to reopen them.

Open points:
- Decide whether the page size should become a request parameter. For now it is fixed so server load is predictable.
- Add per-caller limits on the number of concurrent streams, since each one holds a database cursor.
- Confirm how reviewers see mismatches that were resolved while their stream was open.
- Write a short client guide describing how to resume a stream after a disconnect.
