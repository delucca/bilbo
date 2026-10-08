---
id: 01K84Z4R84NDF5PRSJHJXF5W78
created: 2025-10-22T00:24-03:00
---

# analyst-orders-api: opções consideradas

Anotação rápida das opções que apareceram para o analyst-orders-api, a camada que expõe aos analistas de merchandising as ordens de reposição geradas pelo ShelfSense. Nada aqui está fechado. É um levantamento do que existe, com os prós e contras que deu para enxergar até agora, para não refazer a conversa toda na próxima sessão.

## O que o analyst-orders-api precisa fazer

Em linhas gerais: listar as ordens de reposição sugeridas por loja e por produto, deixar o analista ver o motivo da sugestão (risco de ruptura previsto, estoque estimado, histórico recente), permitir ajustar ou rejeitar uma ordem e registrar quem fez o quê. Depois, entregar as ordens aprovadas para quem executa a compra ou o abastecimento.

O uso é interativo, mas o dado nasce em lote. As previsões e as ordens saem de jobs Spark orquestrados pelo Airflow e caem em tabelas Delta Lake. Então a pergunta central é de onde a API lê e onde ela escreve as edições dos analistas. Quase todas as opções abaixo diferem nisso.

## Opção 1: ler direto do Snowflake

As tabelas de ordens já são publicadas no Snowflake para consumo analítico. Fazer a API consultar o Snowflake direto é o caminho com menos peças novas.

Pontos a favor:
- Não há cópia extra do dado nem pipeline novo.
- Os analistas e as ferramentas de BI já olham para o mesmo lugar, então os números batem.
- Permissões e mascaramento já existem no Snowflake e podem ser reaproveitados.

Pontos contra:
- Latência e custo de warehouse para consultas pequenas e frequentes de uma tela interativa.
- Escrita transacional de edições individuais não é o ponto forte; funciona, mas sem folga para concorrência alta.
- A API fica acoplada ao calendário de carga do Snowflake: se a carga atrasa, a tela mostra dado velho sem avisar.

## Opção 2: banco transacional próprio para o estado das ordens

Manter um banco relacional pequeno só para o estado de trabalho: ordens abertas, edições, aprovações, trilha de auditoria. O job de lote grava as sugestões novas ali (ou um sincronizador copia do Delta), e a API só fala com esse banco.

Pontos a favor:
- Resposta rápida e escrita com transação de verdade, boa para edição concorrente por mais de um analista na mesma loja.
- Separa bem o que é sugestão do modelo do que é decisão humana.
- Facilita impor regras de estado (uma ordem aprovada não volta a rascunho sem registro).

Pontos contra:
- Mais um sistema para operar, com backup, migração e monitoramento.
- Precisa de sincronização de volta para o Delta e o Snowflake, senão a análise de acerto das previsões perde as decisões dos analistas.
- Risco de divergência entre o banco e o lote quando o job roda de novo para o mesmo dia.

## Opção 3: servir a partir do Delta Lake

Ler as tabelas Delta diretamente, seja por um motor SQL sobre o lake, seja por um serviço que leia os arquivos. Evita o Snowflake no caminho da API.

Pontos a favor:
- A fonte é a mesma que o job de lote escreve, sem atraso de publicação.
- Custo de armazenamento baixo e histórico versionado de graça.

Pontos contra:
- Latência de leitura pior para consultas pontuais, a menos que haja cache ou um motor dedicado.
- Escrever edições pequenas em Delta gera muitos arquivos pequenos e conflitos de commit; não é um bom lugar para estado interativo.
- Controle de acesso por linha (por exemplo, analista só vê as lojas da sua bandeira) fica mais manual.

Na prática isso só parece razoável como caminho de leitura, combinado com outra coisa para escrita.

## Opção 4: híbrido, leitura analítica e escrita transacional separadas

Juntar as ideias anteriores: a leitura volumosa (explicações, histórico, comparações) vem do Snowflake ou do Delta, e o estado editável vive num armazenamento transacional pequeno. A API junta os dois na resposta.

É a que melhor encaixa no uso, mas também a que tem mais risco de complexidade. Perguntas que ficaram em aberto:
- Quem é a fonte da verdade quando a sugestão do modelo e a edição do analista divergem?
- Como a API mostra que uma ordem foi regerada por um novo lote depois de editada?
- Qual a política quando o job reprocessa um dia já aprovado?

## Estilo da API e orquestração

Independente do armazenamento, ainda sobra escolher o formato da interface.

- REST simples com paginação e filtros por loja, categoria e status. É o mais fácil de testar e de integrar com o que os analistas já usam. Tende a gerar muitas chamadas quando a tela precisa de ordem mais explicação mais histórico.
- GraphQL para a tela montar o que precisa numa chamada só. Resolve o excesso de chamadas, mas traz custo de esquema, cache e controle de custo de consulta.
- Exportação em lote (arquivos) para quem só quer a lista aprovada. Pode existir ao lado da API, não no lugar dela.

Para o handoff das ordens aprovadas, duas linhas: a API chama o sistema de compras de forma síncrona, ou publica um evento e outro componente entrega com reenvio. A segunda isola falhas externas, mas exige idempotência clara para não duplicar ordem.

Quanto à linguagem, o resto do projeto é Scala, então um serviço em Scala reaproveita modelos e bibliotecas do pipeline. Outras linguagens seriam possíveis, mas hoje não há motivo forte para sair daí.

## Autenticação, auditoria e pontos a decidir

Pontos que valem para qualquer opção:
- Autenticação pelo provedor de identidade da empresa, com escopo por rede de lojas.
- Trilha de auditoria de toda alteração, com quem, quando e o valor anterior.
- Versionamento da API, porque os analistas usam planilhas e scripts que quebram fácil.
- Observabilidade: mostrar na resposta a idade do dado, para o analista saber se o lote do dia já entrou.

Próximos passos possíveis, sem ordem fixa: conversar com alguns analistas sobre o quanto de edição concorrente acontece de verdade, medir o custo de consulta interativa no Snowflake com dados reais, e esboçar o modelo de estado das ordens para ver se o banco próprio se paga. Nenhuma escolha foi feita para o analyst-orders-api ainda.
