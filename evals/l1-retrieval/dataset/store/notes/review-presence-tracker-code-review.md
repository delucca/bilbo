---
id: 01KH1Q8EBN99X89DK8GHNBTTXS
created: 2026-02-09T14:30-03:00
---

# Revisão do presence-tracker: handle_diff/2 idempotente

Revisão de código do presence-tracker. A conclusão principal: `handle_diff/2` precisa ser idempotente. O motivo é que diffs duplicados, entregues depois de um netsplit, contaram em dobro os leaves. Ou seja, o mesmo leave aplicado duas vezes reduzia (ou marcava) a contagem de presença duas vezes, e o número de participantes online ficava errado para produtores e community managers que acompanham o evento ao vivo.

## Conclusão da revisão

`handle_diff/2` deve produzir o mesmo estado final tanto ao receber um diff uma vez quanto ao receber o mesmo diff várias vezes. Hoje isso não vale: diffs duplicados após um netsplit double-counted leaves. A revisão tratou isso como bug de correção, não como detalhe de performance. A regra fica registrada aqui para quem mexer no presence-tracker depois: não aceitar mudança em `handle_diff/2` que dependa de o diff chegar só uma vez.

## O que aconteceu

Depois que o cluster Elixir se partiu e voltou a se juntar, os nós trocaram estado de presença e reenviaram diffs que o outro lado já tinha aplicado. O presence-tracker aplicou tudo de novo. Joins repetidos não causaram dano visível, porque entrar duas vezes na mesma chave não muda o conjunto. Os leaves repetidos foram o problema: cada um era somado ao resultado, então a contagem divergiu da realidade.

## Por que duplicatas acontecem

- Netsplit seguido de reconciliação entre nós Phoenix, com reenvio de diffs.
- Reconexões de clientes WebSocket, que geram eventos de entrada e saída em sequência.
- Entrega pelo menos uma vez entre processos, sem garantia de ordem entre nós.

Não dá para eliminar duplicatas na origem. A correção precisa estar no consumidor.

## Direção da correção

Aplicar o diff como operação sobre conjunto, não como contador incremental. Um leave só tem efeito se a entrada ainda existe no estado; se já saiu, é ignorado. O mesmo vale para join de entrada já presente. A contagem deve ser derivada do conjunto de entradas, nunca mantida por soma e subtração separadas.

```elixir
# esquema, não é o código real
def handle_diff(diff, state) do
  # aplicar joins e leaves como operações de conjunto
  # repetir o mesmo diff devolve o mesmo state
end
```

## Testes pedidos

- Aplicar o mesmo diff duas vezes e comparar o estado com a aplicação única.
- Simular netsplit com reenvio de diffs e checar que a contagem final bate com o número real de participantes.
- Leave de entrada que nunca existiu não pode alterar nada.
- Ordem invertida de join e leave para a mesma chave deve ser tratada sem contagem negativa.

## Pontos em aberto

- Decidir se o estado de presença persistido no CockroachDB precisa da mesma garantia ou se só a cópia em memória importa.
- Verificar se o front em Next.js assume que a contagem nunca diminui e se precisa se proteger disso.
- Medir o custo de manter o conjunto completo em eventos muito grandes.

## Status

A conclusão da revisão está fechada: `handle_diff/2` deve ser idempotente. A implementação e os testes acima ainda estão por fazer.
