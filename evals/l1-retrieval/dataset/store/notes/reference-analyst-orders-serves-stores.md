---
id: 01KSK359AFTNMSE3FZPVRTME92
created: 2026-05-26T18:30-03:00
sources:
  - "code: src/main/scala/shelfsense/api/Routes.scala"
---

# analyst-orders-api reference

This is a working reference for analyst-orders-api, the HTTP service in ShelfSense that lets merchandising analysts at grocery chains read the replenishment orders the platform has generated for a store. It is written quickly and kept general on purpose. Where a detail is not pinned down here, check the code or the deployed config before relying on it.

The one fact to remember: analyst-orders-api serves `GET /v1/stores/{store_id}/orders` over HTTPS on port `8443`. Everything else in this note is context around that.

## What the service is for

ShelfSense predicts store-level stockouts and turns those predictions into replenishment orders. The prediction and order generation happen in batch, in Spark jobs that write to Delta Lake and are scheduled by Airflow. Analysts do not want to read Delta tables directly, and they do not all have warehouse access. analyst-orders-api is the read path for them: it answers the question "what orders does the system currently propose or hold for this store".

It is a read service. It does not generate orders, it does not recompute forecasts, and it does not push anything back into the batch pipeline. If an analyst wants a different order, the change goes through whatever override or review process the product has, not through this API.

The typical consumers are the analyst-facing UI, a few internal scripts that analysts or engineers wrote for spot checks, and sometimes a downstream export that wants a store's orders as a list. All of them go through the same endpoint.

## The endpoint

There is one route that matters for day-to-day use:

```
GET /v1/stores/{store_id}/orders
```

The `{store_id}` segment is a path parameter. It is the store's identifier as the rest of ShelfSense knows it, not a display name and not a chain-level code. The route returns the orders for that single store. There is no cross-store listing on this route; if you need many stores you call it once per store, or you go to the warehouse side.

The response is JSON. It holds a collection of orders for the store, and each order carries enough information for an analyst to see what is being replenished, how much, and where in its lifecycle it is. Exact field names are defined by the service code and its schema; do not copy field names from this note, read the schema.

The version prefix in the path is part of the contract. A breaking change to the response shape would mean a new version prefix rather than a silent change under the existing one. Additive changes can land under the existing prefix, so clients should ignore fields they do not know.

## Transport and port

The service speaks HTTPS only, and it listens on `8443`. There is no plain HTTP listener to fall back to. A client that tries plain HTTP against that port will fail at the handshake or get a protocol error, and that failure is about the scheme, not about the service being down.

When something cannot connect, check in this order:

- The scheme is `https`, not `http`.
- The port is `8443`, not a default web port.
- The host resolves from where the client runs. Analyst laptops and cluster workloads may resolve different names or go through different ingress.
- The certificate chain is trusted by the client. Internal CA issues are the most common cause of TLS errors from scripts.

A service that is up but unreachable on `8443` is usually a network policy or ingress problem, not an application problem. Rule out the network before touching the code.

## Request and response shape

A call is an ordinary authenticated GET. The path carries the store, and any narrowing of the result is done with query parameters rather than a request body. Whatever filters exist, such as limiting by order status or by date, are defined in the service code. Treat the list of supported filters as something to look up, not something to remember from here.

Responses follow the usual conventions: a success returns the JSON collection; a store that exists but has no orders returns an empty collection rather than an error; a store that is not known returns a not-found style response. Distinguishing "no orders" from "unknown store" matters when debugging, because analysts often report the first as if it were the second.

If the collection can be large, the service may page it. Clients should not assume everything arrives in one response. Follow whatever paging mechanism the response advertises instead of guessing at it.

## Authentication and access

Calls need to be authenticated. Analysts get access through the organization's normal identity setup, and the service checks that the caller is allowed to see the requested store. Chains are separate customers, so a caller from one chain must never see another chain's stores. That check is the most important behavior in the service after correctness of the data itself.

When a caller gets an access-denied response for a store they believe they should see, the cause is almost always a mapping of the caller to chains or stores that is out of date, not a bug in the route. Look at how the caller's entitlements are populated before changing any service code.

Service-to-service callers should use their own credentials and not borrow an analyst's. That keeps audit trails honest and keeps rate limits per caller meaningful.

## Where the data comes from

The orders the API returns are produced upstream. The flow, in short:

- Spark jobs written in Scala compute stockout predictions per store and item and derive replenishment orders from them.
- Those results are written to Delta Lake tables.
- Airflow schedules and sequences the jobs, so freshness depends on the DAGs finishing.
- Some serving or reporting data is also available in Snowflake, which analysts and BI tools use for broader analysis.

The API reads from the serving copy of the order data, not from the raw intermediate tables of the Spark jobs. The practical consequence: what the API shows is exactly as fresh as the last successful publish of order data, not as fresh as the last forecast computed. If a batch run failed or is late, the API keeps serving the previous state without complaining.

Because of this, a very common support question, "the API shows old orders", is answered by looking at the Airflow run for the relevant pipeline, not by restarting the service.

## Freshness and consistency

Orders change in discrete steps when a batch publishes, not continuously. Two calls close together will normally return the same thing. Two calls on either side of a publish may return different sets for the same store, and the service does not try to make that appear atomic across stores. One store can be updated while another is still on the previous run.

Analysts comparing the API output to a Snowflake query should expect small differences if the two sources were refreshed at different moments. Before treating a difference as a defect, check when each side was last loaded.

The service may cache. If it does, the cache lifetime is short relative to the batch cadence, and it exists to protect the backing store from repeated identical reads rather than to hide stale data. If a result looks stale after a known publish, a cache is a plausible cause, but check the publish first.

## Operating notes

Things that tend to matter when running analyst-orders-api:

- It is stateless from the application point of view, so replicas can be added or removed without coordination. Scaling is about read volume, which is usually modest and clusters around the start of the analysts' working day.
- Health and readiness are separate concerns. A replica can be alive but not ready if it cannot reach its backing data. Orchestration should route traffic based on readiness.
- Logs are the first place to look for a failing request. Include the store identifier and the caller when asking someone else to investigate, but do not paste credentials.
- Restarting is rarely the fix. Most incidents trace to upstream data, entitlements, TLS or network, in roughly that order of frequency.

Deployments should be rolled gradually so that a bad build does not take out every replica at once. Because the contract is versioned in the path, rolling forward and back should be safe for clients as long as the response shape under the existing version prefix does not change in a breaking way.

## Common problems

### Connection refused or TLS failure

Usually the wrong scheme or port. The service is HTTPS on `8443`. Verify both before looking anywhere else. If the scheme and port are right and TLS still fails, the client likely does not trust the internal certificate authority.

### Empty list for a store that should have orders

First confirm the `{store_id}` value is the identifier used by ShelfSense and not some other code the chain uses. Then check whether the last batch published for that store. A store can have no proposed orders legitimately, for example when nothing is predicted to run out.

### Access denied

Check the caller's entitlements for the chain and store. Do not widen access in the service to make a single complaint go away.

### Old data

Look at the Airflow run history for the pipeline that publishes orders. If the latest run failed or is still running, the API is behaving as designed. Fix or rerun the pipeline.

### Unexpected fields or a missing field

Clients should tolerate new fields. A field that disappears from a response under the same version prefix is a bug and should be raised with the service owners, not worked around in clients.

## Change guidelines

When changing this service, keep these in mind:

- The route `GET /v1/stores/{store_id}/orders` is relied on by the UI and by scripts that nobody on the team may remember writing. Do not rename it or change its meaning. If a new shape is needed, add a new version prefix and run both for a while.
- Keep the service read-only. If a feature seems to need a write, it probably belongs in the order generation or review flow instead.
- Tenant isolation checks belong on every code path that touches store data, including any new filter or export variant. Add the check first, then the feature.
- Do not make the API depend on the Spark jobs directly. It reads published data only. That separation is what lets batch runs fail without taking the API down.
- Keep the listener on HTTPS. Do not add a plain HTTP port for convenience, even internally.

## Related components

Analyst-orders-api sits at the end of a chain, so problems rarely start in it. The pieces around it:

- The Spark jobs in Scala that produce predictions and orders.
- The Delta Lake tables those jobs write, and the published serving copy of the order data.
- The Airflow DAGs that schedule the jobs and publish results.
- Snowflake, where a copy of order data is available for analysis and reporting.
- The analyst UI and scripts that call the endpoint.

If you are new to the project, read the pipeline side first, then come back to this service. Knowing when and how orders are published explains most of the API's behavior.

## Quick reference

- Component: analyst-orders-api
- Route: `GET /v1/stores/{store_id}/orders`
- Transport: HTTPS only
- Port: `8443`
- Access: authenticated, checked per chain and store
- Data: published order data from the batch pipeline, so as fresh as the last successful publish
- First suspects for any complaint: scheme and port, the store identifier, entitlements, then the upstream Airflow run

## Open questions

Things this note does not settle and that someone should confirm and write down next time they look:

- The exact list of query parameters the route accepts and how paging works in detail.
- The cache behavior in front of the backing store, if any, and how long it holds.
- Which hostnames clients should use from each environment.
- Who owns entitlement data and how quickly changes reach the service.

When any of these is confirmed, update this note rather than creating a second one on the same component.
