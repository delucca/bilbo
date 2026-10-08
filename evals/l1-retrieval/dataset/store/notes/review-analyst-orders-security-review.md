---
id: 01KTGK5C8A9YDRVNZDCPK8YNRT
created: 2026-06-07T05:28-03:00
sources:
  - "doc: API Security Review"
---

# Security review of analyst-orders-api

This note records the outcome of the security review of analyst-orders-api, the service merchandising analysts use to look at and change replenishment orders that ShelfSense generates. The short version: every request must carry the `X-Analyst-Token` header, and tokens expire after 12 hours. The rest of this note is the context around that, what it means for callers, and what to check when something breaks. Nothing here was run in this session; it is written from the review's conclusions.

## Naming

The service is called analyst-orders-api in docs and design notes. People and some configs also call it `ordersvc`, which is just short for `analyst-orders-api`. They are the same thing. If you see `ordersvc` in a log line, a deployment label or a chat thread, it is this component and not a second service. In this note the full name is used throughout.

## Scope of the review

The review looked at how analysts and tools reach analyst-orders-api, what a caller can read or change once connected, and how long a credential stays useful if it leaks. It did not cover the Spark jobs that compute stockout predictions, the Delta Lake tables behind them, the Airflow DAGs that schedule runs, or Snowflake access used for reporting. Those have their own controls and are out of scope here.

## Main conclusion

Every request to analyst-orders-api must carry the `X-Analyst-Token` header. There is no anonymous read path and no exception for health-looking or read-only calls as far as the review is concerned. A request without the header, or with an invalid one, is to be refused. The reasoning was simple: order data drives real purchasing at grocery chains, and even read access reveals which stores are expected to run short, which is commercially sensitive.

## Token lifetime

Tokens expire after 12 hours. After that a token is no longer accepted and the caller has to obtain a fresh one. The review picked a window that covers a normal analyst working session without forcing constant re-login, while keeping the damage from a leaked token bounded to a short period. A long-lived token that sits in a shell history or a notebook was the main worry.

## What callers must do

Send `X-Analyst-Token` on every call, not just the first one in a session. Do not rely on a connection staying open to carry identity. Clients should treat a refusal due to an expired token as a signal to get a new token and retry once, not to loop. Tools that run longer than a working day, such as a scheduled export, need a way to refresh rather than a token pasted in once.

## Header spelling

The header name is `X-Analyst-Token`, with that capitalisation and those hyphens. HTTP header names are case-insensitive on the wire, but keep the documented spelling in code, docs and test fixtures so searches find every use. Do not invent variants such as a shorter name or an Authorization-style replacement; the review did not approve any alternative.

## Expiry behavior to expect

When a token passes the 12 hours mark, the next request with it should fail as unauthenticated. Analysts will notice this as a sudden failure in the middle of work, often at the start of a new day if a browser tab was left open overnight. That is expected behavior and not an outage. Support should first ask when the token was issued before digging into the service.

## What the review did not change

The review did not change the order data model, the approval steps for replenishment orders, or the way ShelfSense generates the orders. It only set the rule about the header and the lifetime. Role and store-level permissions inside the service are a separate matter and were left as they were.

## Risks that motivated it

Three concerns came up. First, tokens copied into scripts and left there. Second, shared tokens used by several analysts, which makes the audit trail useless. Third, long sessions on shared workstations in store back offices. The header requirement plus a short lifetime addresses the first and third directly; the second is addressed by issuing tokens per person.

## Logging and secrets

Token values must not be written to logs, error messages or traces. Log that a token was missing, expired or invalid, but never the value. Be careful with request dumps in debugging: the header will be in them. When pasting a failing request into a ticket or chat, strip the token first.

## Testing guidance

Tests for analyst-orders-api should cover a request with no header, a request with a bad value, a request with a valid token, and a request with an expired token. Use a controllable clock rather than waiting out real time, so the expiry check at 12 hours can be tested quickly. Fixtures should use obviously fake token values.

## Operations notes

On call, a spike in refusals right after the start of the working day in a region is normal when tokens from the previous day expire. A spike that is not tied to that pattern deserves a look: it may be a client that stopped refreshing, or a deployment that broke header handling, for example a proxy that strips unknown headers. Check that any gateway in front of analyst-orders-api passes `X-Analyst-Token` through unchanged.

## Gateways and proxies

Any load balancer, reverse proxy or API gateway between the client and analyst-orders-api must forward the header and must not cache responses across different tokens. If a proxy caches by URL alone, one analyst could receive another analyst's response. This is easy to miss when a new layer is added, so it is worth a line in the deployment checklist.

## Open questions

The review left a few things for follow-up: whether a refresh mechanism should exist so long-running tools do not need people to paste a new token, whether revocation before the 12 hours elapse is needed for lost devices, and whether rate limits should be keyed on the token. None of these is decided. Do not assume any of them in code until someone records a decision.

## Impact on other components

Airflow tasks or Spark jobs that call analyst-orders-api to push or read orders need the same header and the same handling of expiry. A task that fetches a token at the start of a long run may find it stale partway through. Design such tasks to get a token close to when it is used. Snowflake-based reports that join order data should not depend on calling the service directly.

## How to answer common questions

Which header carries identity? `X-Analyst-Token`. How long is a token good for? 12 hours. Is `ordersvc` a different service? No, it is short for `analyst-orders-api`. Can a read-only call skip the header? No. Is a different header acceptable? Not without a new review.

## Follow-ups

Update client libraries and docs so the header name and lifetime are stated once and linked from elsewhere. Add the gateway pass-through check to the deployment checklist. Decide on refresh and revocation, then record each as its own decision note and link it back here.
