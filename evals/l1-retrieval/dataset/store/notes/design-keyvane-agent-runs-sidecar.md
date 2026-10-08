---
id: 01KFY7RVAYG522E0JMGK17ADDJ
created: 2026-01-26T19:46-03:00
sources:
  - "code: cmd/agent/writer.go"
---

# keyvane-agent design

keyvane-agent runs as a sidecar next to the application it serves. It writes the current X.509-SVID to `/var/run/keyvane/svid.pem` with file mode `0400`. That file is the only place an application needs to look to get its workload identity. This note records how the agent is shaped, why, and what to watch for. It is written fast, so treat it as a working note and not a spec.

## Names and history

The component is called `keyvane-agent`. Its previous name was `sidecarx`. If you find `sidecarx` in old branches, dashboards, runbooks or chat logs, it means `keyvane-agent`. Do not use the old name in new code, docs or alerts.

The issuing side was renamed too. `mintd` was the previous name of `keyvane-issuer`; the component is called `keyvane-issuer` now. The agent talks to `keyvane-issuer` when it needs a fresh credential, so old references to `mintd` in agent config or comments should be read as `keyvane-issuer`.

Both renames are finished in the product. Leftovers in comments, metrics prefixes and log fields may still exist. Fix them when you touch the code nearby, not in a big sweep.

## What the agent does

The job is small on purpose. The agent runs as a sidecar, gets an identity for the workload it sits beside, keeps that identity fresh, and puts it on disk where the application can read it. Rotation happens without a redeploy of the application. The application only has to reread the file.

The agent does not make authorization decisions. It does not hold policy. It only proves who the workload is and passes along the short-lived credential it was given.

## SVID file output

The agent writes the current X.509-SVID to `/var/run/keyvane/svid.pem` and sets the file mode to `0400`. Read-only for the owner, nothing for group or others. The mode is set on every write, not only the first one, so a rotated file never ends up more open than the old one.

Writes are done by creating a temporary file in the same directory, setting the mode, and renaming it over the target. A reader therefore sees either the old SVID or the new one, never a half-written file. Keep the temp file in the same directory, otherwise the rename is not atomic.

```text
sidecar: keyvane-agent
output:  /var/run/keyvane/svid.pem
mode:    0400
issuer:  keyvane-issuer
```

## Identity and trust

Identity is SPIFFE. The SVID carries the SPIFFE ID of the workload. Services check it during mTLS handshakes, so the agent's output is what makes mutual authentication work between services.

The agent authenticates to `keyvane-issuer` over mTLS as well. For the first credential it needs some bootstrap proof of what workload it is attached to. After that it can use its current SVID to ask for the next one. If the current one has already expired, it falls back to the bootstrap path.

## Rotation behaviour

The agent renews well before expiry, not at expiry. The exact fraction of the lifetime is a config value and is not fixed in this note. The reason for early renewal is simple: a failed attempt then has time to be retried while the old SVID is still valid.

On renewal failure the agent keeps the old file in place and retries with backoff. It does not delete the SVID on error. An expired file on disk is better than a missing file for debugging, and consumers fail the handshake anyway when it is expired.

Secrets from Vault follow the same rule of thumb: short lifetimes, renewal ahead of time, and no application restart. The SVID file is the model the other outputs copy.

## Operational notes

Where the agent runs matters for the path. The directory holding the file must exist and be writable by the agent, and it must be readable by the application's user. Because the file mode is `0400`, the application must run as the same user as the agent, or the file owner must be set to the application's user. This is the most common setup mistake.

When the application cannot read the SVID, check the owner first, then the mode, then whether the directory is a shared volume between the two containers.

State that has to survive restarts, such as coordination data, lives in etcd on the server side, not in the agent. The agent itself is stateless apart from the file it writes, so a restart just fetches a new SVID.

## Failure modes

- Issuer unreachable: the agent keeps serving the old file and retries. Alert on how long the file has gone without refresh, not on a single failed attempt.
- Clock skew: a skewed host may think a valid SVID is expired or not yet valid. Check time sync before anything else.
- Wrong owner on the output directory: the write fails and the agent logs it. The old file stays.
- Stale old name in config: a setting that still names `mintd` or `sidecarx` will not match anything. Rename it.

## Open questions

- Whether to expose a small local status endpoint for the application to learn the expiry time, or leave that to reading the certificate itself.
- Whether the agent should signal the application when the file changes, or whether polling or file watching on the application side is enough. Right now it is left to the application.
- Whether to support more than one output file per agent for apps that want a key and chain split. Not needed yet.

## Do not do

- Do not loosen the file mode to make a consumer work. Fix the ownership instead.
- Do not log the contents of the SVID file or the private key.
- Do not bring back the names `sidecarx` or `mintd` in new work.
