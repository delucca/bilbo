---
id: 01KEXEF2SF6E2DHRS42VES2AJF
created: 2026-01-14T02:08-03:00
---

# carrier-sync-job: carrier API rate limit findings

Testing against the carrier APIs showed that the busiest carrier API allows 60 requests per minute. The carrier-sync-job has to throttle its calls to stay under that. This note records what we saw, why it matters for FreightWeave, and what the carrier-sync-job needs to do about it. It is a research note, not a design: the throttling mechanism itself is still to be written up separately.

## What we found

The busiest carrier endpoint set is the one most dispatchers depend on, because it covers the largest share of regional truck and rail legs. When we pushed calls at it in testing, it accepted traffic up to 60 requests per minute and refused or delayed traffic beyond that. The other carriers we tried allowed more headroom, so the busiest one is the binding constraint for the whole carrier-sync-job.

The limit is per minute, not per second. Short bursts can look fine and then the rejections show up once the minute fills. That makes it easy to miss in a quick manual test and easy to hit in a real sync run.

## Why it matters

The carrier-sync-job pulls carrier status and schedule data that feeds route planning and load rebalancing. If the job overruns the limit, calls fail in clusters, and the data in Redis goes stale for exactly the carrier with the most traffic. Stale carrier data means the OR-Tools planner works from old delay information, and rebalancing decisions get worse right when delays are happening.

A second effect is retry pressure. If failed calls are retried immediately, they add to the load against the same limit and make the situation worse. Any retry logic in the carrier-sync-job has to count against the same budget as first-time calls.

## What the carrier-sync-job must do

- Throttle outbound calls to the busiest carrier so the rate stays within 60 requests per minute, with some margin below the limit rather than right at it.
- Keep the throttle per carrier, since other carriers have different limits and should not be slowed down to match the busiest one.
- Share the budget across all workers or instances of the job. A per-process limiter is not enough if more than one copy runs at once. Redis is the natural place to hold the shared counter, since the project already uses it.
- Count retries and backoff attempts against the same budget.
- Back off when a carrier signals it is throttling us, instead of continuing at the same pace.

## Open points

We have not confirmed whether the limit is a fixed window or a sliding window. This changes how much margin we need. A fixed window can allow a burst at the boundary of two windows, while a sliding window does not. Until that is known, assume the stricter case and spread calls evenly.

We also have not checked whether the limit is per API key or per account. If it is per account, every service that talks to that carrier shares the budget, and the carrier-sync-job may not be the only consumer. That should be asked of the carrier or tested with a second key.

## Related pieces

The carrier-sync-job publishes updates that other FreightWeave components consume through Google Cloud Pub/Sub. A slower sync for the busiest carrier means those updates arrive later, so downstream consumers should not assume a fixed freshness. The FastAPI service that dispatchers use should show the age of carrier data where it matters.

## Next steps

Decide on the limiter design and where its state lives, then write it up as a design note. After that, add a test that drives the carrier-sync-job against a fake carrier enforcing the same limit, so a regression shows up before it reaches a real carrier. Re-test the limit with the carrier periodically, since carriers can change their quotas without much notice.
