---
id: 01KTM0T26K7HZA6RQSKCQP0N48
created: 2026-06-08T13:24-03:00
---

# keyvane-rotator: where things live

This is a map of keyvane-rotator, not a spec. It says where to look first for each piece, in general terms. Check the repo layout before trusting any directory guess here, since names drift.

keyvane-rotator is the Go service that rotates secrets for Keyvane without anyone redeploying the consuming services. It talks to HashiCorp Vault for the secret material, keeps coordination state in etcd, and identifies itself and its peers with SPIFFE identities over mTLS.

## Entry point and wiring

The main package sits under the usual Go command directory of the repo. It parses configuration, builds the Vault client, the etcd client and the mTLS server and client settings, then starts the rotation loop and the API listener. If you want to know what depends on what, read the wiring there first. It is short and mostly constructor calls.

## Configuration

Configuration is loaded once at startup from a file plus environment overrides. The struct definitions live in a config package. Defaults and validation are in the same place. If a setting seems ignored, check validation first, because some fields are normalized or rejected there. Example config files for local runs are kept near the deploy material, not in the config package.

## Rotation core

The rotation logic is the heart of the component. It lives in an internal package named for rotation or scheduling. Look for three things there: the scheduler that decides what is due, the executor that performs one rotation, and the policy types describing how often and how a given secret rotates. Keep these separate when reading; bugs usually come from the executor, while surprises about timing come from the scheduler.

## Vault integration

All Vault access goes through one wrapper package. Callers elsewhere should not import the Vault API client directly. The wrapper handles authentication, token renewal, and the read and write calls for the secret engines in use. Dynamic credential issuance and static secret rewriting take different paths, so find which one a given secret type uses before debugging.

## etcd state and locking

Coordination lives in a store package backed by etcd. It covers leader election or per-secret leases, so two instances do not rotate the same secret at once, and it records rotation status that other parts read. Key layout and prefixes are defined as constants in that package. Change them with care, because running instances share the keys.

## Identity and mTLS

SPIFFE identity handling is in its own package. It fetches and refreshes workload certificates and builds the TLS configs used by both the server and outbound clients. Authorization checks that map a caller's SPIFFE ID to allowed actions are close by, often in middleware. If a call is rejected, look at the identity-to-permission mapping before blaming the network.

## API surface

The service exposes an API for requesting rotation, checking status and, for operators, pausing work. Handlers are in an API or server package, with request and response types next to them. Handlers should stay thin and call into the rotation core. Any API schema file, if one exists, lives beside the handlers or in a separate api directory.

## Delivery to consumers

After a rotation, consumers need to pick up the new value. The notification or delivery code is its own package. It does not push secrets into applications directly; it signals or makes the new version available so the consumer fetches it. Read this package to understand what a consumer sees during the overlap between old and new values.

## Observability

Logging, metrics and tracing setup is in a small shared package. Metrics names for rotation outcomes are declared there or beside the executor. When investigating a failed rotation, logs from the executor and the Vault wrapper are the most useful, and the stored status in etcd shows the last known state.

## Tests

Unit tests sit next to the code they cover. Integration tests that need a Vault dev instance or an embedded etcd are separated, usually by a build tag or a dedicated test directory. Fakes for the Vault wrapper and the store are shared test helpers, so reuse them instead of writing new mocks.

## Deployment and operations

Container build files, manifests and example configs are in a deploy directory at the top level. Runbooks and design notes, if they exist, are in the docs directory. Check there before changing rotation timing or key layout, since operators may rely on current behavior.

## Open gaps in this map

This note does not list exact package names, because they were not verified here. When you confirm one while working, update this note with the real location rather than adding a second note on the same component.
