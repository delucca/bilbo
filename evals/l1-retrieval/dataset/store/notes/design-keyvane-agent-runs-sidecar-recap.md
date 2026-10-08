---
id: 01M33Y7G6ZDBW6BB3E9X4S09TT
created: 2026-09-22T03:53-03:00
---

# keyvane-agent as a sidecar

Rough notes on how keyvane-agent is meant to run next to an application, as a sidecar in the same pod or host group. Written quickly, not checked against anything else we have on this, so there may be overlap.

## Why a sidecar

The app should never talk to Vault directly. keyvane-agent sits beside it, holds the identity, fetches short-lived credentials and hands them to the app. That keeps Vault tokens out of app code and lets us rotate secrets without redeploying the app. Application teams only see a local endpoint or a file.

## Identity

keyvane-agent gets its identity from SPIFFE. It asks the local workload API for an SVID and uses that for mTLS toward the Keyvane control side. The app itself does not need its own certificate for this. If the SVID is not available at startup, the agent waits and retries rather than exiting, because exiting makes the pod restart loop look like an app problem.

## Talking to Vault

After the mTLS handshake the agent authenticates to Vault with the identity it has and asks for leases on behalf of the workload. It keeps the lease bookkeeping in memory. Renewal happens well before expiry, at a fraction of the lease time that is configurable. The usual default has been fine so far.

## Delivery to the app

Two modes: the agent writes secrets to a shared in-memory volume the app reads, or the app calls a local socket. Files are simpler for most teams. The socket is better when the app wants to pull on demand. We should not support both for the same secret at once, it gets confusing about which one is authoritative.

## Rotation without redeploy

When a secret rotates, the agent rewrites the file atomically (write to a temp name, then rename) so the app never reads a half-written value. Apps that cache secrets at boot still need a reload hook. The agent can signal the app or call a configured hook; which one depends on the team. Documenting this is still on the list.

## State and etcd

Anything the agent needs to survive a restart is small. Shared coordination state lives in etcd, not in the agent. The agent should treat etcd as read-mostly and tolerate it being slow. If etcd is unreachable the agent keeps serving the credentials it already has until they expire, then fails closed.

## Failure behaviour

If Vault is down, the agent serves cached credentials until the lease ends, then stops serving them. It does not extend anything on its own. Health is exposed through a readiness check so the orchestrator can hold traffic until the first credentials arrive. Liveness should only fail on a stuck agent, not on a Vault outage, otherwise we restart pods for no gain.

## Resource limits

The agent is light. Memory and CPU requests in the sidecar spec should be small and match what we have measured; the configured limits are in the deployment template. Watch for growth with many leases per workload, that is where memory goes.

## Startup ordering

The app must not start reading secrets before the agent has written them. Options are an init step that blocks until the first fetch is done, or the app retrying on missing files. Init blocking is cleaner. Sidecar start order in the orchestrator is not guaranteed on its own, so do not rely on it.

## Open questions

- Whether one agent per pod is enough for pods with several containers that need different identities.
- How to handle a very short lease when the clock is a bit off between nodes.
- Whether the reload hook should be part of the agent config or of the app manifest.

## Security notes

The shared volume should be memory-backed and readable only by the app's user. Logs from keyvane-agent must never contain secret values or full lease identifiers. Debug logging is off by default and should stay off in production. mTLS peers are checked against the expected SPIFFE ID, not just a CA chain.
