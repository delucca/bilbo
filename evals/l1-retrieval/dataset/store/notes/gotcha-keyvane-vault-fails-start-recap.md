---
id: 01JX21EP2M138J7M8PMEGREBTB
created: 2025-06-06T04:14-03:00
---

# keyvane-vault-plugin fails to start in Vault

Second pass at why keyvane-vault-plugin sometimes will not come up when Vault tries to mount it. Written quickly, from memory of a few bad afternoons, so treat it as partial. Another note probably covers the same ground; merge when someone has time.

## Symptom

Vault says the plugin failed to start or the mount fails outright. The log line is vague, usually something about the plugin process exiting before the handshake finished. Nothing useful shows up in the keyvane-vault-plugin output because the process dies early.

## First thing to check

Is the binary the one Vault expects? Most of the time the problem is a mismatch between what is registered in the plugin catalog and what is actually on disk. Fix that before looking at anything deeper.

## Checksum in the catalog

The catalog stores a hash of the binary. After any rebuild the hash changes and the old registration is stale. Vault refuses to launch it. Re-register with the new hash every time the binary changes, including local dev rebuilds. This is the single most common cause.

## Plugin directory setting

Vault only loads plugins from the configured plugin directory. If the binary sits somewhere else, even a symlink pointing out of that directory, it will not start. Check the config of the Vault server, not the plugin.

## File permissions

The binary needs to be executable by the user Vault runs as. Containers make this worse: the image build may copy the file without the execute bit. Also check ownership, since some setups reject a binary writable by others.

## Architecture mismatch

Building on a laptop and shipping to a Linux node bit us once. The plugin has to be compiled for the target OS and CPU. A wrong build gives an exec failure that looks like a handshake problem.

## Static linking

Go builds with cgo enabled can pull in a libc the Vault image does not have. Build with cgo off for the plugin. The failure looks the same as any other early exit.

## mlock and capabilities

Vault wants to lock memory, and plugin processes inherit some of that. If the container lacks the capability, startup can break in confusing ways. Not fully sure this applies to the plugin itself, so verify on your own setup before blaming it.

## TLS handshake between Vault and plugin

Vault and the plugin talk over a local connection secured with a one-time certificate. If the environment is slow or the clock is off, the handshake can time out. It is not the same as the mTLS the plugin uses for its own upstream calls, so do not mix them up.

## Upstream mTLS config

keyvane-vault-plugin reads its own client cert and trust bundle on start. If those files are missing or unreadable at mount time, it exits at once. The error is easy to miss because it goes to stderr, which Vault may swallow unless debug logging is on.

## SPIFFE identity not ready

The plugin needs a workload identity before it can talk to the issuer. If the SPIFFE agent socket is not available yet when Vault mounts the plugin, startup fails. On node boot, ordering matters: the agent has to be up first. Retrying the mount after a short wait usually works.

## etcd reachability

Some state lives in etcd. If the endpoints are unreachable, or auth to etcd is wrong, the plugin can fail during init instead of degrading. It does not wait long. Check connectivity from the same network namespace Vault uses, not from your shell.

## Config values at mount time

Missing required fields in the mount config also kill startup. We saw this after renaming a field. Compare against the last known good config and look for typos before anything else.

## Debugging approach

Turn on debug logs for Vault. Run the plugin binary by hand to see whether it prints anything; it should complain that it is meant to be launched by Vault, which at least proves the binary runs. Then work down: catalog hash, directory, permissions, architecture, upstream certs, identity, etcd.

## Open questions

- Whether the plugin should wait and retry for identity and etcd rather than exit.
- Whether the stderr output can be surfaced better in Vault logs.
- Whether we should add a startup self-check that names the failing dependency.

## Notes for later

If this happens again after an upgrade of Vault, check the plugin protocol compatibility first. A newer server may drop support for older plugin builds, and the fix is a rebuild against the current SDK.
