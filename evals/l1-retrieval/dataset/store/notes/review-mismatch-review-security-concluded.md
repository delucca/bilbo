---
id: 01K6HY0R547BB5ENGBEVZRE5R4
created: 2025-10-02T04:43-03:00
sources:
  - "doc: Review API security review"
---

# mismatch-review-api security review

Security review of mismatch-review-api, the service that finance operations reviewers use to look at flagged mismatches between card-processor settlement files and internal ledger entries. The review concluded that mTLS is mandatory for every caller, and it found 2 endpoints missing the authorization interceptor. Both findings are open work until the fixes land and are re-checked. This note records what the review settled, why, and what a later session should do about it.

## Name of the component

The component used to be called flagdesk. It is called mismatch-review-api now. Older material may still say flagdesk: tickets, dashboards, alert rules, Terraform resource names, log fields, runbooks and chat history. When you meet that name, read it as mismatch-review-api. The rename was about the name only. The responsibilities stayed the same: it serves reviewers, exposes the mismatches the matching pipeline produced, and records the reviewer's decision.

In this note the component is always mismatch-review-api. If you search the repo or the infrastructure code for leftovers, search for flagdesk as well. A stale reference to the old name can hide a place where the security settings were never updated, which matters for the findings below.

## What the review covered

The review looked at how mismatch-review-api is reached and who it lets in. It covered transport security on the gRPC surface, the way callers are identified, the authorization step that decides what an identified caller may do, and how those pieces are deployed through Terraform. It also looked at the data the service can reach in PostgreSQL, since a reviewer-facing service sits next to settlement and ledger data that is sensitive for a marketplace.

The review was a read-through of the service definition, the interceptor chain and the deployment configuration. It was not a penetration test and did not try to break anything at runtime. Treat the findings as what a careful reading showed, not as proof that nothing else is wrong.

## Finding: mTLS is mandatory

The main conclusion is that mTLS is mandatory for mismatch-review-api. Every client has to present a certificate that the service verifies, and the service has to present its own. Plain TLS with server-only authentication is not enough, and neither is a plaintext listener, even inside the private network.

The reasoning is plain. The service exposes financial mismatch data and lets callers change review state. Network position alone is a weak guarantee, because other internal services and tooling share the same network. Requiring client certificates means that identity is established at the connection, before any request logic runs, and the authorization step can rely on a verified peer identity instead of a header someone typed.

Practical consequences: no environment may run mismatch-review-api with the mutual check switched off, including local development shortcuts that could be copied into shared environments. Test and staging should use the same mode as production, with their own certificate authority, so the code path is exercised everywhere.

## Finding: 2 endpoints missing the authorization interceptor

The review found 2 endpoints without the authorization interceptor. The rest of the gRPC methods go through the interceptor, which checks that the verified caller is allowed to perform that specific method. These 2 endpoints skip it, so any caller that completes the mTLS handshake can use them, whatever role it holds.

That is a real gap even with mTLS in place. mTLS proves who the caller is. It does not say what the caller may do. A service identity that was only meant to read mismatches could use the unprotected endpoints in ways it was never granted.

The likely cause is that the interceptor is attached per method or per service registration rather than as a default for the whole server, so methods added later were easy to miss. That is a guess from how the review read the code. Confirm it when fixing.

## Why a default-deny interceptor is the better fix

Do not fix this by adding the interceptor to just the 2 endpoints and moving on. That repeats the same weakness: the next method someone adds can be missed again. The better fix is to install the authorization interceptor once, at the server level, so every method passes through it, and to make a method with no declared permission fail closed.

Under that design a new endpoint is denied until someone states which roles may call it. An endpoint that is meant to be open, such as a health check, has to be listed explicitly as exempt, and the list should be short and reviewed. This turns a silent omission into a visible failure at the first test run.

## What to check when fixing

Identify the 2 endpoints from the review's own list, not from memory. Then check what each one does: whether it reads or writes, and what data it returns. If either one writes review decisions or exposes ledger detail, treat the gap as more urgent than the read-only case.

After the change, add a test that walks every registered method on the server and asserts that each one is either covered by the interceptor or on the explicit exemption list. That test is the thing that stops the problem from coming back. Also add a test that a caller with a valid certificate but the wrong role is refused on both of the formerly open endpoints.

Check the old name too. Any permission table, role mapping or certificate subject that still says flagdesk should be updated to mismatch-review-api, or the interceptor may deny everything, or worse, match the wrong rule.

## Deployment and Terraform notes

The mTLS requirement has to be enforced in configuration as well as in code. The Terraform that deploys mismatch-review-api should provision the server certificate, the trust bundle for client certificates, and the rotation path. It should not leave the mutual check as an optional flag with a permissive default. A reviewer of the Terraform change should look for a default that quietly weakens the setting.

Certificate rotation is a risk in its own right. If rotation fails, the service either stops serving or someone is tempted to disable verification to get back up. The runbook should say plainly that disabling verification is not an allowed recovery step, and give the real recovery path. Alert on certificate expiry well before it happens.

Resource names in Terraform may still carry the old name flagdesk. Renaming them can force replacement of resources, so plan that change on its own and read the plan before applying.

## Callers and identities

With mTLS mandatory, every caller needs its own identity. The reviewer-facing frontend, any batch tooling and any other internal service that reads mismatches should each get a distinct certificate, not share one. Shared identities make the authorization interceptor meaningless, because it cannot tell the callers apart, and they make audit trails useless.

The Kafka side matters here too. The matching pipeline publishes mismatches over Apache Kafka, and mismatch-review-api serves them to reviewers. That pipeline link is outside this review, so do not assume it is covered by these conclusions. If the service also consumes or produces on Kafka, its credentials there need their own look.

## What this review did not settle

It did not assess the PostgreSQL side in depth: database roles, row-level restrictions and whether the service account has more rights than it needs. It did not review audit logging, meaning whether every review decision is recorded with the verified caller identity. It did not review rate limiting or abuse handling on the gRPC surface. Each of these deserves its own note or follow-up review.

It also did not verify the fixes. Until someone re-runs the check against the changed code and configuration, both findings stay open.

## Follow-ups

First, move authorization to a server-wide default-deny interceptor, with a short explicit exemption list. Second, add the test that enumerates every method and checks coverage. Third, confirm that all environments run with mTLS required, and that the Terraform has no weaker default. Fourth, clean out remaining flagdesk names from permissions, certificate subjects and alert rules, and plan any Terraform resource rename separately. Fifth, schedule the narrower reviews listed above for database access and audit logging.

When the fixes are done, update this note: say which of the findings are closed, how that was verified, and what the exemption list contains in general terms. Remove statements here that no longer hold instead of stacking corrections beneath them.
