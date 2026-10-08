---
id: 01K7CDNPHSZZG3JKYSXE7WDVFA
created: 2025-10-12T11:37-03:00
---

# dispatch-api request models forbid extra fields

The dispatch-api request models set `extra="forbid"` in Pydantic v2. We did this because misspelled optional fields were silently ignored and produced wrong plans. A dispatcher would send an optional field with a typo, the API would drop it, and the planner would run on defaults. The plan came back looking valid but did not reflect what the dispatcher asked for. With `extra="forbid"`, an unknown field now makes the request fail validation instead.

## Decision

Every request model in dispatch-api uses `extra="forbid"` in its Pydantic v2 model config. Unknown fields are rejected, not ignored. This applies to request bodies only. The reason is the one above: misspelled optional fields were silently ignored and produced wrong plans.

## Context

FreightWeave plans multi-leg truck and rail routes and rebalances loads when delays occur. Regional freight dispatchers use it. dispatch-api is the FastAPI service they and their tools call to ask for routes and rebalances. Behind it sit OR-Tools for the solving, Redis for state, and Google Cloud Pub/Sub for delay events.

Most request fields are optional, with defaults the solver can work with. That is what made the silent drop so harmful. A missing optional field is legal, so a misspelled one looked the same as an omitted one.

## The failure we saw

Pydantic's default is to ignore fields it does not know. A client sent an optional constraint under a slightly wrong name. FastAPI accepted the request, the model dropped the unknown key, and the solver ran without the constraint. The route was feasible, just not the route the dispatcher wanted.

Nothing in the logs showed it. The request was valid by the model's rules, and the response was a normal plan. People found it only by comparing the plan with what they had meant to send.

## What the setting looks like

The setting goes in the model config of each request model:

```python
model_config = ConfigDict(extra="forbid")
```

This is the Pydantic v2 style of config. Do not use the old inner `Config` class from Pydantic v1 for this.

## Why forbid and not warn

We considered logging a warning when unknown fields appear and keeping the lenient behavior. We rejected that. Nobody reads warnings on a request path, and the wrong plan still goes out. A hard failure reaches the caller right away, at the point where the typo can be fixed.

## Why not rely on client-side checks

Clients are written by different teams and some are scripts that dispatchers edit by hand. We cannot count on all of them validating their payloads. The server is the one place that sees every request, so the check goes there.

## Behavior change for callers

A request with an unknown field now fails validation, and FastAPI returns its usual validation error response. The error names the offending field. Callers that used to send extra keys and get away with it will start seeing failures. That is intended. Those keys were doing nothing, or were typos of real fields.

## Rollout notes

Before turning this on for a model, check what real clients send. Look at recent request logs for fields that are not in the model. Some may be harmless leftovers from older client versions. Tell the owners of those clients before the change ships, so they can drop the keys.

The change is easy to revert, since it is one config line per model. But reverting brings back the silent-drop problem, so do not do it just to quiet a noisy client. Fix the client.

## Scope: which models

The rule covers the request models of dispatch-api. When someone adds a new request model, it gets `extra="forbid"` from the start. A reviewer should flag any new request model without it.

If many models end up repeating the same config line, a shared base model is a reasonable cleanup. Nobody has done that yet, and it is not part of this decision.

## Scope: what it does not cover

This decision is about inbound request models. It does not say anything about response models or about the shape of Pub/Sub messages. Delay events from Pub/Sub are a separate input with its own parsing, and they may need different tolerance, since the producers are not the dispatchers' tools. Decide that separately.

## Testing

Each request model should have a test that sends a payload with a misspelled optional field and expects a validation failure. The test should assert that the request is rejected, not that the field is dropped. This guards against someone removing the config line while refactoring.

Also keep a test that a correct payload with only the optional fields it needs still passes. We do not want `extra="forbid"` to turn into a requirement that every field be present.

## Gotchas

- Nested models need the setting too. Pydantic does not carry `extra="forbid"` from an outer model to inner ones. A typo inside a nested object is still ignored if the inner model lacks the setting.
- Subclasses inherit the model config from their parent, so a shared base with `extra="forbid"` covers them. A model that sets its own config can override this, so check.
- Field aliases matter. If a model accepts a field under an alias, a client sending the other name will be rejected. That is correct, but it can look like a bug at first.
- Case differences count as unknown fields. A key with the wrong capitalization is rejected.

## Alternatives considered

- Keep the default and add a post-validation check that compares the raw body keys with the model fields. This is more code, it can drift from the model, and it duplicates what Pydantic already does.
- Make the optional fields required. This would catch the typo, because the real field would then be missing, but it makes every request heavier for dispatchers and defeats the purpose of having defaults.
- Use a schema linter on clients only. This helps, but it does not protect us from clients we do not control.

## Consequences

Plans now reflect what was actually sent, or the request fails loudly. The cost is that clients with sloppy payloads break and need fixing. We accept that cost, because a wrong plan costs dispatchers more than a rejected request does.

## Open questions

- Whether to add a shared base model for all request models.
- Whether Pub/Sub delay events should be strict too.
- Whether the validation error text should include a hint about the nearest valid field name, to help with typos.

## How to check this still holds

Open the request models in dispatch-api and confirm each one, nested ones included, has `extra="forbid"`. Then send a request with a misspelled optional field and confirm it is rejected. If it is accepted and the field is dropped, the setting has been lost somewhere.
