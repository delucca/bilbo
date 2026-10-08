---
id: 01KQ0MA42Z7FSQX400W62WSK4K
created: 2026-04-24T17:53-03:00
---

# config-scanner: duração do scan completo após batching por região

Este relatório substitui a nota anterior "config scanner september load" e traz o valor novo: depois do batching por região, o `config-scanner` termina o scan completo das 1,840 contas em `27 minutes`, e esse número troca a duração informada antes.

A nota foi escrita com pressa, para o próximo engenheiro ou agente não ter que redescobrir o que mudou. Tudo o que está aqui é qualitativo, exceto a duração e o total de contas.

## Resumo do resultado

O scan completo de 1,840 contas no `config-scanner` leva `27 minutes` com o batching por região ligado. Quem ler só esta nota já tem a resposta: a duração vigente é `27 minutes`, para o conjunto inteiro de contas, de ponta a ponta, da primeira conta lida até o último resultado gravado.

O valor antigo, da nota de carga de setembro, não vale mais. Não use aquele número em planejamento, em SLA interno nem em conversa com o time de segurança.

## O que esta nota substitui

A nota anterior, "config scanner september load", registrava a duração medida antes do batching. Ela foi medida com as contas processadas sem agrupamento por região, e o tempo total era bem maior. Essa nota deve ser tratada como obsoleta. Se alguém ainda a encontrar em uma busca, o valor correto agora é `27 minutes`.

Não copiei o valor antigo para cá de propósito. Assim ele não volta a circular por engano.

## Contexto do AuditMesh

O AuditMesh varre configurações de infraestrutura em nuvem, procura violações de política e gera tickets de remediação. Quem usa são times de segurança de nuvem. O `config-scanner` é o componente que lê as configurações das contas e as avalia contra as políticas escritas para o Open Policy Agent.

O resultado da avaliação alimenta a etapa seguinte, que cria os tickets no Jira. Por isso a duração do scan afeta diretamente quanto tempo passa entre uma configuração errada existir e alguém ver o ticket.

## O que mudou: batching por região

Antes, o `config-scanner` tratava as contas sem agrupar por região. Agora o trabalho é dividido em lotes por região. Cada lote reúne as contas e os recursos de uma mesma região da AWS e é processado como uma unidade.

O ganho vem de reaproveitar chamadas e contexto dentro da região, e de não alternar entre endpoints regionais a cada conta. Também fica mais fácil limitar a concorrência por região sem estourar os limites de taxa das APIs.

## Como a medição foi feita

A medida é o tempo de relógio do scan completo, cobrindo todas as 1,840 contas, do início da execução até a última gravação de resultado. Não é média por conta nem tempo de CPU somado das funções.

O resultado de `27 minutes` vale para o ambiente em que medimos, com as contas que existiam naquele momento. Se o total de contas mudar bastante, a duração muda junto, e esta nota precisa de revisão.

## Papel do AWS Lambda

O `config-scanner` roda em funções AWS Lambda. Com o batching, cada lote vira uma invocação ou um grupo de invocações ligadas a uma região. O tempo máximo de uma função continua sendo uma restrição a respeitar, e os lotes foram desenhados para caber nele com folga.

Se um lote crescer demais, a função pode ser cortada pelo limite de execução. Vale ficar de olho nisso quando entrarem contas novas em uma região já grande.

## Papel do DynamoDB

O estado do scan e os resultados intermediários ficam no DynamoDB. O batching por região também ajudou a distribuir melhor as escritas, porque os itens de uma região são gravados juntos e de forma mais previsível.

O risco conhecido é throttling quando muitas regiões gravam ao mesmo tempo. Até agora isso não apareceu como problema na medição, mas é o primeiro lugar a olhar se a duração subir.

## Papel do Open Policy Agent

A avaliação das políticas no Open Policy Agent não mudou com esta alteração. As regras são as mesmas e o resultado de violações por conta deve ser idêntico ao de antes. O que mudou foi a forma de alimentar o motor com as entradas.

Se alguém notar diferença no conjunto de violações entre o scan antigo e o novo, isso é bug e não efeito esperado do batching.

## Impacto nos tickets do Jira

A criação de tickets no Jira vem depois do scan. Com o scan mais curto, os tickets saem mais cedo. Porém o Jira tem seus próprios limites de taxa, então encurtar o scan não encurta necessariamente a fila de criação de tickets.

O número de `27 minutes` se refere só ao scan, não à criação dos tickets. Não some nem subtraia nada sem medir a etapa do Jira separadamente.

## O que não foi medido

Não há medição aqui do custo em Lambda antes e depois, nem do consumo de capacidade no DynamoDB. Também não medi o comportamento sob falha parcial de uma região. Esses pontos ficam como lacunas conhecidas.

Também não comparei a variação entre execuções diferentes. O valor de `27 minutes` deve ser lido como o resultado do scan completo medido, não como garantia estatística.

## Riscos e pontos de atenção

Regiões muito desbalanceadas podem fazer um lote dominar o tempo total. Nesse caso o scan inteiro espera pelo lote mais lento, e a duração total é ditada por ele.

Outro ponto é a ordem de processamento dos lotes. Se uma região lenta começar tarde, ela estica o tempo final. Começar pelas regiões maiores é uma ideia a testar.

## Como reproduzir ou conferir

Para conferir, rode um scan completo com o batching por região ativo e compare o tempo de relógio total com `27 minutes`. Use o mesmo conjunto de 1,840 contas ou registre quantas contas havia, para a comparação fazer sentido.

Se o resultado ficar muito diferente, procure primeiro mudança no número de contas, throttling no DynamoDB e lotes que bateram no limite de tempo do Lambda.

## Próximos passos

Medir a etapa de criação de tickets no Jira em separado. Medir custo antes e depois. Testar a ordenação dos lotes por tamanho de região. Registrar a variação entre várias execuções, e não só uma.

Quando houver medida nova, atualizar esta nota em vez de criar outra sobre o mesmo assunto.

## Resumo final para quem chegou com pressa

O `config-scanner` com batching por região termina o scan de 1,840 contas em `27 minutes`. Isso substitui o valor da nota antiga "config scanner september load". A avaliação das políticas não mudou, e a etapa do Jira não está incluída nesse tempo.
