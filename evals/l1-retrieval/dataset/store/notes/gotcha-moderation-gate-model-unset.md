---
id: 01KZQ0GAH4BVE3Y2KQJ37GPE9Y
created: 2026-08-10T20:35-03:00
sources:
  - "code: lib/townhall/moderation/config.ex"
---

# moderation-gate crashes at boot without MODERATION_MODEL_URL

When MODERATION_MODEL_URL is unset, moderation-gate crashes at boot with `** (RuntimeError) environment variable MODERATION_MODEL_URL is missing`. It does not start in a degraded mode and it does not fall back to a default model. The process dies during startup, before it accepts any WebSocket connection, so no moderation happens at all. Treat that line in the logs as meaning one thing: the variable was never set in the environment of the process that launched moderation-gate.

## Symptom

A deploy or a local start of moderation-gate exits almost at once. The log shows the RuntimeError above and nothing useful after it. Because the Phoenix side of TownHall Pulse keeps running, producers can still see polls and Q&A in the Next.js frontend, but nothing is being screened. Questions can look stuck in "pending review" or, depending on how the gate is wired in that environment, simply never show up for moderators. The first thing people check is the database, and CockroachDB is fine. Do not lose time there.

## Cause

The model endpoint is read from the environment at boot and the code raises when it is missing. This is a deliberate hard failure: a gate that silently lets everything through is worse than one that does not start during a live event. The variable is expected to hold the URL of the moderation model that moderation-gate calls to score incoming questions and poll comments.

The usual ways it ends up unset:

- A new environment (staging copy, preview, a developer laptop) was created without copying the variable.
- The variable was set in the shell but not passed into the container or release process that actually runs moderation-gate.
- A secrets or config update replaced the whole set of variables and dropped this one.
- A typo in the name. The name is exact and case sensitive, so a near miss counts as unset.

## Check and fix

Confirm the variable is visible to the process, not just to your shell. In a running container or release, inspect the environment of that process. Then set it to the model URL for that environment and restart moderation-gate.

```sh
export MODERATION_MODEL_URL="<model endpoint for this environment>"
```

The value in the example is a placeholder. Use the real endpoint from the team's secrets store for the target environment, never one copied from another environment without checking that it points to the right model.

## Gotchas around it

- An empty string may behave differently from a fully unset variable depending on how the config reads it. If the crash persists with the variable present, check that it is not blank and has no stray quotes or trailing whitespace.
- Restarting is required. The value is read at boot, so changing it in the environment of a running moderation-gate has no effect until it restarts.
- Do not "fix" this by catching the error and continuing. The crash is the intended guard.

## Before a live event

Add the variable to the pre-event checklist for every environment that will run moderation-gate. After a deploy, look for the boot line in the logs and confirm moderation-gate stays up and starts handling traffic. If the RuntimeError shows up during an event, set `MODERATION_MODEL_URL` first, restart, and only then look at anything else.
