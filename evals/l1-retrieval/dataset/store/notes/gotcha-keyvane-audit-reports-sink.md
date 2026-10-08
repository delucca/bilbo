---
id: 01JRDHG5203R2BZ6WQAM3QDFYH
created: 2025-04-09T12:08-03:00
---

# keyvane-audit-log fails when the log volume fills up

When the /var/log/keyvane volume fills up, keyvane-audit-log stops being able to write to its sink and reports `audit: sink write failed: no space left on device`. This is the first thing to check if audit entries stop showing up, or if credential issuance and rotation start failing or slowing down for no obvious reason. The message looks like a generic disk error and is easy to dismiss as noise from the host. It is not noise: audit records are being lost or blocked.

## Symptom

The error line is `audit: sink write failed: no space left on device`. It comes from keyvane-audit-log itself, not from Vault, etcd or the SPIFFE layer. You will usually see it repeated, once per failed write, so the log around it can get very loud. Other components may look healthy at the same time, which is what makes it confusing. The cause is the volume mounted at /var/log/keyvane, not anything in the credential path.

## Why it matters

Audit output is part of the security guarantee. A security engineer relying on the trail for review will find a gap exactly where the volume was full. Depending on how the sink is configured, the service may either drop entries or block callers while it waits for the write. Check which behaviour your deployment has before assuming nothing was lost. Treat any window with this error as a period with an incomplete audit trail and say so in the incident notes.

## What to check first

- Look at free space and inodes on the /var/log/keyvane volume. Running out of either gives the same message.
- Find what is using the space. Usually it is audit files that were never rotated or shipped off the host, or another process writing to the same volume.
- Check whether log shipping is stuck. If the shipper is down, local files pile up until the disk is full.
- Confirm the error stops after space is freed. If it keeps going, the sink may need a restart of keyvane-audit-log to reopen its files.

## Fixing and preventing it

Free space by moving or compressing old audit files that have already been shipped. Do not delete files that have not been confirmed as shipped, since they may be the only copy. Once space is back, check that new entries are being written and note the time range that may be missing.

To keep it from recurring, alert on disk usage for the /var/log/keyvane volume well before it is full, and alert on the shipper falling behind. Size the volume for the longest outage of the shipper you are willing to tolerate. Keep this volume separate from anything else that can write large amounts of data, so an unrelated process cannot starve the audit sink.

## Open questions

Worth confirming and recording here once known: whether the sink drops or blocks on write failure in our deployments, whether entries buffered in memory survive until space returns, and whether keyvane-audit-log recovers on its own after space is freed or needs a restart.
