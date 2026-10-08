---
id: 01JTWVBGV0AJZFTFZCJBTH2B2T
created: 2025-05-10T07:20-03:00
sources:
  - "doc: Keyvane agent troubleshooting guide"
---

# keyvane-agent fails on a stale trust bundle

keyvane-agent refuses to talk to the Keyvane servers and logs `x509: certificate signed by unknown authority` when the trust bundle at `/etc/keyvane/bundle.pem` is stale. The agent itself is fine in that case. The bundle on the host no longer contains the CA that signed the certificate the server is presenting now, so the mTLS handshake is rejected on the agent side. It looks like a network or Vault problem at first. It is not.

## Symptom

The agent starts, tries to reach the server, and fails the handshake. In the logs you see `x509: certificate signed by unknown authority`. No credentials are issued and no rotation happens for the services that depend on that agent. Applications that already hold a short-lived credential keep working until it expires, then start failing too. That delay is why the first reports often come well after the actual breakage.

Typical things that make people look in the wrong place:

- The error mentions a certificate, so people suspect the workload's SPIFFE identity. The identity is usually fine.
- The server side shows few or no useful errors, because the handshake dies before any request arrives.
- Vault is healthy and etcd is healthy. Neither is involved in this failure.

## Cause

The trust bundle is a PEM file at `/etc/keyvane/bundle.pem`. keyvane-agent reads it to decide which CAs it trusts when it verifies the server certificate. If the CA that issues server certificates is rotated or replaced and the file on the host is not updated, the agent has an old set of roots. The new server certificate chains to a CA the agent has never heard of, and Go's TLS stack returns the unknown authority error.

Common ways the bundle goes stale:

- A CA rotation happened and the new bundle was never distributed to this host.
- The host was built from an old image that carried an old bundle, and nothing refreshed it afterward.
- The file was copied by hand once and then forgotten.
- A config management run failed or was skipped on that host, so it kept the old file.

## How to confirm

Check the file before changing anything else. Look at its modification time and at which CAs it holds, and compare with the bundle that the other, working hosts have. If the working hosts have a newer file or different CAs, you have found it.

Also confirm the error is really the trust problem and not a different TLS failure. A hostname mismatch or an expired certificate gives a different message. Only the unknown authority message points to the bundle.

A quick way to see what is in the file, using only the path from this note:

```sh
openssl crl2pkcs7 -nocrl -certfile /etc/keyvane/bundle.pem | openssl pkcs7 -print_certs -noout
```

This prints the subject and issuer of each certificate in the bundle. Compare the issuer of the server certificate with the subjects listed.

## Fix

Replace `/etc/keyvane/bundle.pem` with the current bundle from the source of truth for your environment, then restart keyvane-agent so it reloads the file. Do not assume the agent picks up a changed file on its own; restart it and watch the logs to see that the error is gone.

Do not work around this by turning off certificate verification. That removes the whole point of the mTLS setup and hides the next real problem. If the new bundle still fails, the issue is something else, such as the server presenting a chain that is missing an intermediate certificate.

## Prevention

- Treat the trust bundle as something that is distributed and versioned, not as a one-time file copy.
- When a CA rotation is planned, ship the new bundle to every host first and rotate the server certificates afterward. Keep the old and new CAs together in the bundle during the overlap.
- Add a check that compares the bundle on each host with the current one, and alert on a difference before a rotation, not after.
- Alert on the agent logging `x509: certificate signed by unknown authority`, so the failure is seen on the first occurrence and not when downstream credentials expire.

## Notes for later

If this error shows up on many hosts at once, suspect a rotation that went out in the wrong order, with certificates changed before bundles. If it shows up on one host, suspect that host's bundle only. Either way, start with `/etc/keyvane/bundle.pem` and not with Vault, etcd or SPIFFE.
