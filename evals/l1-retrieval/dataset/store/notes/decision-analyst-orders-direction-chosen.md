---
id: 01KY0F7HETT567341WNQ5CKA9F
created: 2026-07-20T16:14-03:00
---

# analyst-orders-api: direção geral escolhida

Anotação rápida sobre o rumo que o time escolheu para o `analyst-orders-api`. Não é um contrato fechado, é a direção geral. Os detalhes finos ficam para quando a gente implementar de fato.

O `analyst-orders-api` é a porta de entrada que os analistas de merchandising usam para ver, ajustar e aprovar os pedidos de reposição que o ShelfSense gera a partir da previsão de ruptura por loja. A discussão era basicamente: a API deve calcular coisas na hora ou só servir o que o pipeline já deixou pronto? A gente decidiu pela segunda opção.

## Decisão

O `analyst-orders-api` não calcula previsão nem monta pedido. Ele lê o que o pipeline em Spark já produziu e grava só as intervenções do analista (ajuste, aprovação, rejeição). A lógica de previsão continua toda no lado batch, em Scala, e a API fica fina.

## Por que assim

- Previsão dentro da API deixava a latência imprevisível e duplicava regra de negócio que já existe no pipeline.
- Com a API só lendo, fica mais fácil explicar para o analista de onde veio cada número.
- Se a previsão for reprocessada, a API não precisa saber como; ela só enxerga o resultado novo.
- Os analistas já esperam que o pedido seja do ciclo do dia, não em tempo real.

## Dados e fronteiras

Os pedidos gerados chegam por tabelas Delta Lake publicadas pelo pipeline, orquestrado pelo Airflow. A leitura para a API vem de uma camada já exposta no Snowflake, para não pesar o cluster Spark com consulta interativa. As edições dos analistas são guardadas separadas do pedido original, como uma camada de sobreposição. Assim o pedido sugerido nunca é sobrescrito e dá para comparar sugestão contra decisão humana depois.

Um ponto que a gente quer manter: o pipeline é dono do pedido sugerido, a API é dona da intervenção. Nenhum dos dois escreve no espaço do outro.

## Pontos em aberto

- Como tratar conflito quando o pipeline republica um pedido que o analista já tinha mexido. A tendência é preservar a edição e sinalizar a diferença, mas isso ainda não foi fechado.
- Como expor o histórico de edições sem poluir a resposta principal.
- Quais limites de uso fazem sentido; ainda falta olhar o comportamento real dos analistas.

## Esboço da separação

Só para fixar a ideia, o fluxo é este:

```text
Spark (Scala) -> Delta Lake -> Snowflake -> analyst-orders-api
analyst-orders-api -> edicoes do analista (camada separada)
```

## Próximos passos

Validar essa separação com um ou dois analistas antes de mexer em contrato de resposta. Se aparecer necessidade forte de recalcular algo na hora, a gente volta nessa decisão em vez de enfiar cálculo na API aos poucos.
