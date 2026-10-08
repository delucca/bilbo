---
id: 01KJQAY9E0BYM53HXXV4506TQP
created: 2026-03-02T10:14-03:00
---

# Retry limit for the sync worker

The sync worker retries a failed push 3 times before it gives up.

## Setting

`sync.max_retries = 3` in `/etc/alpha/sync.toml`.
