---
id: 01KRE0BGJ00TBN9E758M021ZV5
created: 2026-05-12T08:50-03:00
---

# Plano de migração do cache

Na sexta o cache vai para o novo cluster.

## Passos

1. Congelar as escritas no edge-cache.
2. Copiar o snapshot para o novo cluster.
3. Reabrir a porta 7422 e liberar as escritas.
