---
id: 01M2240HPC2GYVNE0B5265JY0T
created: 2026-09-09T00:40-03:00
---

# Decisão: load-rebalancer também move cargas em pátio

Esta nota substitui a nota anterior "load rebalancer only moves", que dizia que o rebalanceador só movia cargas planejadas; o valor novo é que o load-rebalancer agora também move cargas com `status=AT_YARD`.

## Decisão

O load-rebalancer agora também move cargas com `status=AT_YARD`. Isso substitui a regra antiga, que era mover apenas cargas planejadas. A razão é que cargas que estão no pátio podem ser reatribuídas sem precisar chamar de volta um caminhão. Então elas são candidatas legítimas quando uma rota atrasa e é preciso redistribuir.

Resumindo para quem lê só esta nota: antes, só carga planejada entrava no conjunto de movimentação. Agora, carga planejada e carga com `status=AT_YARD` entram. O resto do comportamento do load-rebalancer não mudou.

## Por que a regra antiga era limitada

A regra de só mover cargas planejadas era conservadora. Uma carga planejada ainda não foi fisicamente comprometida com um veículo, então mexer nela é barato. O problema é que, durante um atraso, muitas cargas úteis para absorver a folga estavam paradas no pátio e ficavam fora da otimização. O solver (OR-Tools) via menos opções do que existiam de fato, e os despachantes acabavam reatribuindo essas cargas na mão.

## Por que cargas no pátio são seguras de mover

Uma carga no pátio não está dentro de um caminhão em trânsito. Reatribuí-la significa apenas trocar o veículo ou o trecho futuro ao qual ela pertence. Nenhum caminhão precisa voltar, e nenhuma viagem em andamento é interrompida. Por isso o custo operacional é parecido com o de uma carga planejada, e o ganho para o rebalanceamento é real.

## Pontos de atenção

- O filtro de elegibilidade do load-rebalancer precisa tratar `status=AT_YARD` como elegível junto com o status de planejada. Qualquer outro filtro que ainda assuma "só planejadas" fica inconsistente com esta decisão.
- Cargas em outros estados, como as já em trânsito, continuam fora do rebalanceamento. Esta decisão não as inclui.
- Se o estado da carga for lido do Redis, vale conferir que o status do pátio está atualizado antes de rodar o rebalanceamento. Um status velho pode fazer o solver mover uma carga que já saiu.
- Eventos de atraso chegam pelo Pub/Sub e disparam o rebalanceamento. Nada muda nesse fluxo; muda só o conjunto de cargas consideradas.

## Como verificar

Criar um cenário de atraso com pelo menos uma carga em `status=AT_YARD` e uma planejada, rodar o load-rebalancer e conferir que as duas podem ser reatribuídas. Depois repetir com uma carga em trânsito e conferir que ela continua intocada.

## Histórico

A nota anterior, "load rebalancer only moves", fica substituída por esta. Se alguém encontrar referência à regra de mover só cargas planejadas, tratar como desatualizada e apontar para esta decisão.
