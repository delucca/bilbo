---
id: 01KPP32E001CXHGCTYMYW7R80C
created: 2026-04-20T15:40-03:00
---

# Retry limit raised

This replaces decision-retry-limit. Pushes were failing during short outages, so `sync.max_retries` is now 5.
