---
id: 01JY5CCTDHGHRVC9199HEJ108E
created: 2025-06-19T21:39-03:00
---

# keyvane-vault-plugin structure

keyvane-vault-plugin is the piece of Keyvane that runs inside HashiCorp Vault as a secrets engine plus an auth backend. It is a Go program that Vault starts as a separate process and talks to over its plugin RPC channel. Vault owns storage, audit and policy; the plugin owns the logic for turning a workload identity into a short-lived credential and for rotating the secrets behind it. Keep that split in mind when changing anything: if you find yourself reimplementing something Vault already does, stop.

## Layout

The code is split into a few layers, top to bottom.

- Backend entry: builds the Vault backend object, registers the path handlers, declares which paths need the seal-wrap or are unauthenticated, and wires the periodic and rotation hooks.
- Path handlers: one file per group of paths (roles, credential issue, rotation, config). They parse and validate the request, check the role, and call into the service layer. They should stay thin.
- Service layer: the actual issuing and rotating logic. It does not know about Vault request types, which keeps it testable without a running Vault.
- Clients: small wrappers for the things the plugin talks to outside Vault, mainly etcd and the SPIFFE workload API. Each is behind an interface so tests can swap in a fake.

## Identity and trust

Callers prove who they are with mTLS, and the identity that matters is the SPIFFE ID in the client certificate. The auth side takes that ID, matches it against role definitions, and hands Vault a token with the policies the role allows. The matching rules between SPIFFE IDs and roles are not described here; see [[keyvane-spiffe-bridge-maps]] for how that mapping is organised. The plugin does not mint its own trust roots. It reads the bundle it is given and checks chains against it.

## State

Vault storage holds role definitions, plugin config and lease bookkeeping. etcd holds the state that other Keyvane components also need to see, such as rotation coordination and the current version pointer for a secret. Do not copy etcd data into Vault storage as a cache without thinking about staleness; the two stores can disagree briefly and the code assumes etcd is the source for the shared parts.

Issued credentials are leases. Revocation and renewal go through the normal Vault lease path, and the plugin only implements the revoke callback that tears down the backing credential.

## Rotation

Rotation is driven by a periodic function in the backend plus an explicit rotate path that operators can call. The flow is: take a coordination marker in etcd, create the new secret version, switch the pointer, then retire the old one after consumers have had time to pick up the new one. This is what lets services rotate without a redeploy, so the ordering matters and the retire step must never run before the pointer switch is visible.

## Things to watch

- Plugin restarts happen when Vault is upgraded or reloads the plugin, so no in-memory state may be the only copy of anything.
- Errors from etcd should surface as retryable to the caller; errors from validation should not.
- Logging goes through the Vault logger and must never include secret material or full certificates.
- Tests for the service layer use the fake clients; tests for the handlers use the Vault logical test harness.
