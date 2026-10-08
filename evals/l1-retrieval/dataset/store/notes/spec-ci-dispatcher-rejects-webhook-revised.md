---
id: 01KYYWH4Y5F8K797G954EJPG7Y
created: 2026-08-01T11:44-03:00
---

# ci-dispatcher request body limit

This note replaces the earlier note "ci dispatcher rejects webhook"; the current value is that ci-dispatcher sets `maxBodySize` to `5 MB`, up from the old limit of 1 MB.

## What the setting does

`maxBodySize` is the largest request body that ci-dispatcher will accept on its incoming webhook endpoint. Anything larger is refused before the payload is parsed, so nothing downstream sees it. The value is now `5 MB`. The earlier limit was 1 MB, and that smaller number is what caused the rejections described in the old note.

If you are reading logs or old tickets that mention the 1 MB cap, treat them as describing behavior that no longer applies. A body that used to be refused for size may now go through.

## Why it changed

Webhook payloads from GitHub Actions and from the repositories we watch can get large when a single upgrade pull request touches many packages. A monorepo bump with a long list of changed files, check results and commit metadata can go past the old cap. When that happened, ci-dispatcher refused the delivery and the targeted test run for that pull request never started. From the outside it looked like the dispatcher had silently dropped the event.

Raising the cap to `5 MB` covers the large payloads we have seen with room to spare, without turning the limit off. The limit is still there on purpose, so a broken or hostile sender cannot push an arbitrarily large body into the Node.js process.

## Things to check when a webhook is still refused

- Confirm the running ci-dispatcher actually picked up the new `maxBodySize`. A container built before the change keeps the old value until it is rebuilt and redeployed with Docker.
- Look at the real size of the failing delivery. If it is above `5 MB`, the limit is working as designed and the sender needs to send less, or we need a deliberate new decision.
- Check any reverse proxy or load balancer in front of the dispatcher. Those have their own body limits and can refuse a request before ci-dispatcher sees it. Raising `maxBodySize` does nothing for that case.
- Make sure the sender is not retrying the same oversized delivery in a loop. Repeated refusals can fill the delivery history and hide the real problem.

## Effects on the rest of the system

Larger accepted bodies mean more memory held per request while the payload is read and parsed. At the current volume that is fine, but it is worth remembering if many large deliveries arrive at once. Payload data we keep in SQLite is not stored whole; only the fields the dispatcher needs are written, so database growth should not track the body limit.

The GitHub Actions workflows that trigger on dispatcher output are not affected by the change. They get the same trimmed job description as before.

## Open points

- We have not decided whether `maxBodySize` should be configurable per repository. For now it is one global value.
- A clearer error for oversized bodies would help. Today the sender sees a plain refusal and has to guess the reason from the status. A message that names the limit would save time.
- If payloads keep growing, the better fix may be to ask senders for a smaller event shape, not to raise the cap again.

## History

The original cap of 1 MB was a default chosen early on, before large multi-package upgrades were common. The old note about the dispatcher rejecting webhooks recorded the symptom and the old limit. This note is the current reference for the setting and supersedes it.
