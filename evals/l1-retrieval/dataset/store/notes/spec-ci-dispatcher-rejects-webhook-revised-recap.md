---
id: 01M3SGG9CM7YMVP8G17XTTD5CF
created: 2026-09-30T12:56-03:00
---

# ci-dispatcher rejects webhook (revised notes)

Second pass on why ci-dispatcher turns away some incoming webhooks. Written quickly, not checked against the earlier note, so expect overlap and gaps.

## What it does

ci-dispatcher receives webhook calls from the git host when a dependency upgrade pull request changes. It decides which GitHub Actions workflow to trigger and which targeted tests to run. If the payload does not pass its checks, it refuses the call and nothing gets dispatched.

## Signature check

Every request is checked against the shared secret before anything else is parsed. A mismatch is rejected right away. Most "random" rejections I have seen were a secret that was rotated on one side only.

## Payload size

There is a configured limit on body size. Large pull request events with many changed lockfile entries can go over the usual value. The rejection happens before parsing, so the log says little about the content.

## Event types

Only a small set of event types is accepted. Anything else gets a polite refusal, not an error. Worth remembering when someone enables extra events on the host side and then asks why nothing happens.

## Replay and duplicates

Deliveries that carry an id already seen are dropped. The seen ids are kept in the SQLite store for a limited window. A retry from the host within that window looks like a rejection but is intended.

## Clock and timestamps

If the request carries a timestamp, it must be within the allowed skew. Containers with a drifting clock produce confusing rejections. Check the Docker host time first.

## Rate limiting

Bursts from a single repository are throttled. Platform engineers who merge many upgrades at once hit this. The limit is configurable and the usual value is fine for normal use.

## Revised behaviour

The revision changed when the checks run: cheap checks (signature, size, event type) now come first, and the database lookup for duplicates comes last. Rejections also now return a clearer status instead of a generic failure. I am not sure every path was updated, so verify the unusual ones.

## Logging

Each rejection logs a reason category and the delivery id but not the body. That keeps secrets out of logs. When debugging, ask the host for the delivery record instead of turning on body logging.

## Local testing

Replay a captured delivery against a local instance in Docker with the same secret. Change one thing at a time: secret, size, event type. This quickly shows which check is firing.

## Open questions

- Should the duplicate window be longer for slow retry schedules?
- Are rejections counted anywhere we can alert on?
- Does the throttle apply per repository or per installation? I believe per repository but did not confirm.

## Things to avoid

Do not loosen the signature check to make a test pass. Do not raise the size limit without looking at memory use in the Node.js process.

## Follow-ups

Add a test for each rejection reason. Reconcile this note with the earlier one on the same subject and keep one.
