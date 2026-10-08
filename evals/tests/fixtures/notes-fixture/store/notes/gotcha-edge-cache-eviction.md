---
id: 01KM0DBEF0FH2PWDT59CHH4W9A
created: 2026-03-18T09:05-03:00
---

# edge-cache drops every entry under memory pressure

When the resident size passes 900 MB the cache evicts everything at once and logs `ERR_EVICT_STORM`.

## Fix

Set `cache.high_water = 700MB` so eviction starts early and stays gradual.
