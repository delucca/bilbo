---
id: 01K6FT6BBRTSVTZCMC9F7HVMCB
created: 2025-10-01T08:58-03:00
sources:
  - "doc: Gateway Weekly Report"
---

# fhir-gateway weekly push report

This note records how fhir-gateway behaved over one recent week of production traffic, and what that says about the component. fhir-gateway is the part of ClinicSlotter that takes appointments scheduled by front-desk staff and pushes them upstream as FHIR resources. Its internal codename is `hl7door`, and you will see that name in older logs, dashboards, branch names and a few chat threads. Both names mean the same component. In this note I use `fhir-gateway` everywhere else.

The headline: over one week fhir-gateway pushed `14,300` appointments upstream with a success rate of `99.2%`. The rest of this note explains what that figure covers, how to read it, and what I would check next. It was written quickly from what I had on hand, so treat the interpretation sections as working judgment, not measured fact.

## Headline numbers

For one week, fhir-gateway pushed `14,300` appointments to the upstream system. The success rate across that week was `99.2%`. A push counts as a success when the upstream system accepted the appointment resource and we recorded the acknowledgement. A push that was rejected, timed out, or left in a retry state at the end of the window counts against the rate.

A question like "how many appointments did fhir-gateway push in a week" has the answer `14,300`. A question like "what was the success rate" has the answer `99.2%`. Neither figure is a target. They are what was observed.

## What the component is

fhir-gateway is a boundary component inside the Rails application. It translates our internal appointment records, which live in MySQL, into FHIR appointment resources and sends them to the upstream endpoint a clinic is connected to. It also handles the acknowledgement coming back and writes the outcome onto the appointment so the front desk can see whether the push landed.

It is not a general integration layer. It does one job: outbound appointment pushes. Scheduling logic, clinician availability and room constraints all stay in the core scheduler and are not decided here.

## The codename hl7door

The name `hl7door` came from the early days, when the plan was to speak an older message format at the door of the clinic systems. The plan moved to FHIR but the codename stuck in places. Where I have seen it: log tags, a couple of metric names, an old runbook title, and some Heroku config labels. If you search for `hl7door` and find hits, they are about fhir-gateway. Do not treat it as a separate service.

I would rename the leftovers eventually, but renaming metrics breaks dashboards, so it has been left alone on purpose.

## How a push flows

When an appointment is created or changed, the Rails app enqueues a job on Sidekiq. The job loads the appointment, builds the FHIR resource, and calls the upstream endpoint. On success it stores the acknowledgement. On failure it classifies the error and either retries or gives up and flags the appointment.

Because the work happens in Sidekiq, the front desk never waits on the upstream system while booking. That is the main reason the push is asynchronous, and it is why the success rate is measured after the fact and not at booking time.

## Reading the success rate

The `99.2%` figure is a rate over pushes, not over appointments. If one appointment is pushed several times because it was edited, each push is counted. That means a busy edit pattern could pull the rate in either direction, depending on whether the edits tend to succeed.

It is also a weekly aggregate. A single bad afternoon can hide inside a good week. If someone asks whether the gateway was healthy, the week-level number says mostly yes, but it does not rule out a short outage.

## What counts as a failure

Failures fall into a few groups. Validation rejections from upstream, where our resource was not accepted as written. Transport problems, such as timeouts or connection resets. Authentication problems, where credentials for a clinic had expired or been revoked. And pushes that were still retrying when the week closed.

I did not break the small remainder down by group for this note. That would be the first thing to do if the rate drops, because each group points at a different owner: our mapping, the network, or the clinic configuration.

## Retries and backoff

Retries are handled by Sidekiq's retry mechanism with growing delays. Only errors that look temporary are retried. Validation rejections are not retried, since sending the same bad resource again cannot help. This keeps the queue from filling with pushes that will never succeed.

One thing to watch is that a long upstream outage causes retries to bunch up, and when upstream comes back they all arrive together. We have not seen that cause trouble at current volume, but it is a plausible risk.

## Idempotency

Pushes are meant to be safe to repeat. An appointment carries a stable identifier into the FHIR resource, so a second push updates the same upstream record and does not create a duplicate. This is what makes retrying acceptable. If an upstream system ignores the identifier and creates duplicates, retries would become harmful, so any new upstream connection should be checked for this before going live.

## Volume in context

The weekly volume of `14,300` is modest. It is spread across many small clinics, each contributing a small share, with the heaviest load during working hours on weekdays. Nothing about this volume strains the Sidekiq workers or MySQL on the current Heroku setup.

The low volume also means a single misbehaving clinic can move the success rate noticeably. One clinic with expired credentials for a day can account for a visible part of the failures.

## Operational footprint

fhir-gateway runs as part of the main Rails app and its Sidekiq workers on Heroku. There is no separate deployment to manage. That is convenient, but it also means a deploy of the app restarts the workers that do pushes, and jobs in flight get picked up again afterwards. Restarts are a normal source of a few retried pushes and should not be mistaken for upstream trouble.

## Monitoring

The signals I would look at are the push success rate, the count of pushes waiting in retry, and the age of the oldest unpushed appointment. The last one is the most useful for the front desk, since it says how stale the upstream view could be.

Some of these signals still carry the `hl7door` name. Keep that in mind when building a query or an alert, or you will find an empty result and think the data is missing.

## Known weak spots

The mapping from our appointment model to the FHIR resource is the most fragile part. Clinics use free-text notes and unusual appointment types, and these do not always fit upstream expectations. Most validation rejections trace back to this mapping.

Credentials handling per clinic is the second weak spot. Expiry is easy to miss, and the failure looks like a generic rejection until someone reads the detail.

## What I did not check

I did not verify the week against upstream's own count. The `14,300` figure comes from our side, so it reflects what we sent and recorded, not what upstream stored. I also did not split the rate by clinic or by hour. Those are the obvious gaps in this report.

## Suggested next steps

First, compare our weekly count with the upstream count to confirm nothing is silently dropped. Second, break the failures into the groups above and see which one dominates. Third, look for a clinic that contributes a disproportionate share of failures. Fourth, decide whether to retire the `hl7door` labels in metrics, accepting the dashboard rework that follows.

## Open questions

Is `99.2%` good enough for the clinics? Front-desk staff may care more about specific appointments that never arrive than about the aggregate. Should a failed push be surfaced more loudly in the scheduling screen? And should the success rate be tracked per appointment instead of per push, to avoid the edit effect described earlier? None of these is settled.

## Summary of facts

fhir-gateway, also called `hl7door` internally, pushed `14,300` appointments upstream in one week with a success rate of `99.2%`. It is the outbound FHIR appointment push path of ClinicSlotter, runs inside the Rails app with Sidekiq on Heroku, and retries only temporary errors. The weak points are the resource mapping and per-clinic credentials.
