---
id: 01KQN4ZVE0X5JWFRMKMR7SED1B
created: 2026-05-02T17:10-03:00
---

# importer fails when two imports overlap

Overlapping imports stop with `ERR_ROW_LOCK` on the orders table.

## Fix

Take the advisory lock first: `SELECT pg_advisory_lock(42)`.
