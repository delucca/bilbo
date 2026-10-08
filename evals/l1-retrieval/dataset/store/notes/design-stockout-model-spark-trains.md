---
id: 01K38EH9J97QFQVJCE3MH3184T
created: 2025-08-22T05:32-03:00
---

# Design do stockout-model

O stockout-model é o job Spark do ShelfSense que estima a chance de ruptura (stockout) por SKU em cada loja. Ele treina um `GBTClassifier` por cluster de lojas, roda no Spark `3.5.1` e grava as probabilidades de ruptura por SKU em tabelas Delta. Essas probabilidades alimentam a geração de pedidos de reposição que os analistas de merchandising das redes de supermercado revisam. Esta nota registra o desenho como está hoje e o porquê das escolhas principais. Está escrita com pressa, então não é um documento completo.

## Visão geral do job

O job é escrito em Scala e orquestrado pelo Airflow. Uma execução faz, em ordem: lê as features já preparadas, agrupa as lojas em clusters, treina um modelo por cluster, pontua todos os SKUs das lojas do cluster e escreve o resultado em Delta. Depois da escrita, uma etapa separada leva os dados para o Snowflake, de onde a camada de reposição e os analistas consomem.

O job não gera pedidos. Ele só entrega probabilidades. A regra que transforma probabilidade em quantidade a pedir fica em outro componente e pode mudar sem retreinar nada.

## Por que um modelo por cluster

Lojas muito diferentes (tamanho, perfil de público, frequência de entrega) têm padrões de ruptura distintos. Um modelo único para a rede inteira ficava dominado pelas lojas grandes e errava nas pequenas. Treinar um `GBTClassifier` por cluster dá a cada grupo parecido um modelo próprio, sem chegar ao extremo de um modelo por loja, que teria poucos exemplos positivos e seria caro de manter.

O custo é operacional: há vários modelos para treinar, versionar e monitorar a cada execução. Aceitamos isso porque os clusters são poucos e o treino de cada um é independente, então roda em paralelo.

## Entrada e features

As features vêm de tabelas Delta de vendas, estoque, entregas e calendário promocional. Em linhas gerais, usamos histórico recente de vendas por SKU e loja, nível de estoque observado, atraso de entregas, promoções ativas e sazonalidade simples. Ruptura é um evento raro, e por isso o rótulo é muito desbalanceado. Tratamos isso com pesos de classe, não com reamostragem, para manter as probabilidades razoavelmente calibradas.

A calibração importa mais do que a acurácia bruta. Quem consome o resultado usa a probabilidade como número, então um modelo que ordena bem mas dá valores distorcidos atrapalha a decisão de reposição.

## Saída em Delta

A escrita é por partição de data de pontuação, com uma linha por loja e SKU contendo a probabilidade de ruptura e o cluster de origem. Gravar em Delta nos dá transações atômicas: se o job falha no meio, a tabela continua com a versão anterior completa, e o Snowflake nunca lê meia execução. Também dá para reprocessar uma data específica sobrescrevendo só aquela partição.

Convém guardar junto da saída o identificador do cluster e a data do treino, para que um valor suspeito possa ser rastreado até o modelo que o produziu.

## Operação e armadilhas

- Fixar a versão do Spark em `3.5.1` nos ambientes de teste e de produção. Diferenças de versão mudam detalhes do `GBTClassifier` e podem alterar resultados entre execuções.
- Se um cluster ficar com poucos exemplos positivos, o treino fica instável. Hoje a saída é fundir esse cluster com o mais próximo, e não forçar um modelo ruim.
- Reexecuções do Airflow precisam ser idempotentes: sempre sobrescrever a partição da data, nunca anexar.
- Quando a distribuição das probabilidades muda de repente de um dia para o outro, olhar primeiro as tabelas de entrada (atraso de carga, estoque zerado por falha) antes de culpar o modelo.

## Pendências

Falta decidir como acompanhar a qualidade por cluster de forma contínua, comparando as rupturas previstas com as observadas depois. Também está em aberto se vale recalcular os clusters com mais frequência ou mantê-los estáveis para facilitar a comparação ao longo do tempo. Até lá, os clusters mudam só de forma deliberada e registrada.
