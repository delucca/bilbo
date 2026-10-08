---
id: 01KPQATTNG6H12V8WZH345ZTMT
created: 2026-04-21T03:14-03:00
---

# Plano do delay-monitor: flow control do streaming pull

Esta nota substitui a nota anterior "delay monitor move synchronous". O valor novo do flow control no streaming pull do delay-monitor é `max_messages=500`, no lugar dos 200 planejados antes.

## Contexto

O delay-monitor é o componente do FreightWeave que escuta eventos de atraso de caminhões e trens e avisa o replanejamento de cargas. O codinome interno dele é `tardiwatch`. Nos logs, em dashboards e em conversas de equipe você pode ver os dois nomes; são o mesmo serviço. Nesta nota usamos só `delay-monitor`.

Os eventos chegam por Google Cloud Pub/Sub. O plano é consumir com streaming pull, sem o modelo síncrono que a nota antiga discutia.

## Decisão sobre o flow control

O streaming pull do delay-monitor vai usar flow control com `max_messages=500`. O valor anterior era 200 e está descartado.

Motivo, em resumo: com 200 o consumidor ficava ocioso em picos de atrasos, quando muitas rotas mudam ao mesmo tempo, e o backlog da subscription crescia. Com 500 o consumidor segura mais mensagens em voo e acompanha melhor a rajada.

## O que muda no código

- Ajustar a configuração de flow control do subscriber para `max_messages=500`.
- Conferir se há outro lugar que repete o valor antigo (config, testes, docs) e atualizar.
- Manter o limite em bytes como está, a não ser que o teste de carga mostre problema.

## Riscos a vigiar

Mais mensagens em voo significa mais memória no processo e mais trabalho repetido se o processo cair antes do ack. O tratamento precisa ser idempotente, porque o Pub/Sub pode reenviar.

Também vale checar o prazo de ack. Se o processamento de cada mensagem for lento (consulta ao Redis, chamada ao OR-Tools para rebalancear), 500 mensagens em voo podem estourar o prazo e gerar reentrega em cascata.

## Testes planejados

1. Teste de carga com rajada de eventos de atraso, comparando backlog e latência com o valor antigo.
2. Observar uso de memória do delay-monitor durante a rajada.
3. Simular queda do processo no meio da rajada e ver se as reentregas são absorvidas sem duplicar replanejamentos.

## Pendências

- Definir se o valor fica fixo ou vira configuração por ambiente.
- Registrar o resultado dos testes de carga nesta nota ou em uma nota de pesquisa separada.
- Se o ack começar a estourar, rever o valor antes de aumentar réplicas.
