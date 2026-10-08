---
id: 01JV19PZJ4Q7HBM72K65N327J3
created: 2025-05-12T00:48-03:00
---

# ci-dispatcher spec: webhook intake and body size limit

This note describes what the ci-dispatcher accepts from GitHub and how it hands work on to the rest of PatchPilot. The one hard number to remember: the ci-dispatcher rejects webhook payloads larger than 1 MB, and the limit is set by the configuration key `maxBodySize`. Everything else below is general on purpose; look at the code and the config file for details that are not written here.

## Purpose

PatchPilot opens dependency upgrade pull requests and then needs to run targeted tests against them. The ci-dispatcher is the piece that sits between GitHub Actions and GitHub webhooks on one side and the test runner on the other. It receives an event, decides whether it concerns a PatchPilot pull request, and queues the matching test run. It does not run tests itself.

Platform engineers who maintain many repositories point their repositories at one ci-dispatcher instance, so the intake has to be cheap and has to fail fast on input that is clearly wrong.

## Body size limit

Webhook payloads larger than 1 MB are rejected by the ci-dispatcher. The threshold is not hard-coded; it comes from the configuration key `maxBodySize`. The default value corresponds to 1 MB.

What this means in practice:

- A payload at or below the limit goes on to signature checking and parsing.
- A payload above the limit is refused before it is parsed. The dispatcher does not try to read the rest of the body, so a huge request cannot eat memory.
- The rejection is a client error response, not a server error. GitHub shows the delivery as failed in the webhook delivery log, which is the first place to look when an event seems to vanish.
- Nothing is queued for a rejected payload, and nothing is written to the SQLite store for it.

If you raise `maxBodySize`, think about memory first. Several deliveries can be in flight at once and each body is held in memory while it is checked.

## Configuration

`maxBodySize` lives with the other dispatcher settings and is read at startup. Changing it needs a restart of the process or the container; there is no live reload. In Docker deployments the value is usually passed in through the mounted config file or an environment-backed config layer, so check which of the two wins before assuming a change took effect.

Example of the relevant setting, using only the values from this note:

```json
{
  "maxBodySize": "1 MB"
}
```

The exact accepted format of the value (bytes, string with unit) is whatever the config loader parses. Check the loader before writing a new value by hand, since a value it cannot parse should stop startup rather than silently fall back.

## Request flow

The order of checks matters, because the size check is the cheapest and runs first.

1. Size check against `maxBodySize`.
2. Signature check of the payload against the shared secret.
3. Parse the JSON and read the event type.
4. Filter: ignore events that do not concern a pull request opened or updated by PatchPilot.
5. Record the event in SQLite and queue the targeted test run.

A failure at any step ends the request. Earlier steps never depend on later ones, so a payload that is both too large and badly signed is reported as too large.

## Why a limit exists

Webhook bodies from GitHub are normally small. Large ones tend to come from push events with very many commits or from a misconfigured sender. Neither is useful to PatchPilot, which only needs the pull request identity, the repository, and the head reference. Rejecting early keeps the dispatcher responsive and keeps junk out of the database.

The limit is a guard, not a feature. If legitimate deliveries start to hit it, first narrow which events the repository sends to the ci-dispatcher, and only then consider raising `maxBodySize`.

## Troubleshooting

When a pull request does not get its test run:

- Open the webhook delivery log in GitHub and look at the response for that delivery. A client error with no queued run points at the size limit, among other things.
- Compare the delivery payload size with the current `maxBodySize` setting on the running instance, not just the file in the repository, since a container may be running an older config.
- Check the dispatcher logs for a rejection line mentioning the body size.
- If the payload was under the limit, move on to the signature and the event filter; the size check is not the cause.

## Open points

- The dispatcher does not currently report how close deliveries come to the limit. A metric for the largest accepted body would help decide whether 1 MB is still the right default.
- It is not decided whether different repositories should be able to have different limits. For now there is one value for the whole instance.
- The rejection response body is terse. It could say which limit was hit so that the sender can see it without reading server logs.
