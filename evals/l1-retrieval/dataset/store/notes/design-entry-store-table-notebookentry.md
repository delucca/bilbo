---
id: 01K30DFPDJDFPQVA2FBG82J3TT
created: 2025-08-19T02:40-03:00
sources:
  - "code: db/schema/NotebookEntry.sql"
---

# Esquema do armazenamento de entradas (entry-store-schema)

Nota de design sobre entry-store-schema, o esquema SQL Server que guarda as entradas do caderno eletrônico no LabNotebook Sync. O fato central: a tabela `dbo.NotebookEntry` é particionada pela coluna `CreatedMonth` e tem índice clusterizado em (NotebookId, EntryId). Tudo abaixo parte disso e explica o porquê, as consequências e o que cuidar ao mexer.

## Resumo rápido

Em entry-store-schema, a tabela principal é `dbo.NotebookEntry`. Ela é particionada por `CreatedMonth`. O índice clusterizado é composto por NotebookId e EntryId, nessa ordem. Quem consulta por caderno anda por uma faixa contígua; quem consulta por período se beneficia da eliminação de partições.

## Contexto

O LabNotebook Sync junta entradas de cadernos com a saída de instrumentos e mantém trilha de auditoria. Cientistas escrevem muito e leem por caderno. O pessoal de conformidade lê por período e por caderno, geralmente para auditoria. O esquema foi pensado para esses dois padrões de leitura.

## Tabela dbo.NotebookEntry

É a tabela que representa uma entrada do caderno. Cada linha tem identificação do caderno, identificação da entrada, o mês de criação usado como chave de partição e o conteúdo ou a referência a ele. Anexos grandes e arquivos brutos de instrumento não ficam na linha; ficam no Azure Blob Storage, e a linha guarda só a referência.

## Particionamento por CreatedMonth

A coluna `CreatedMonth` é a chave de partição de `dbo.NotebookEntry`. Ela representa o mês em que a entrada foi criada. Como o valor não muda depois da criação, a linha não migra entre partições em updates comuns. Isso é importante: se alguém passasse a alterar `CreatedMonth`, cada alteração viraria um movimento físico de linha.

## Por que particionar por mês

Três motivos. Primeiro, consultas de conformidade com filtro de período tocam poucas partições. Segundo, a manutenção de dados antigos (reindexação, arquivamento, compressão) pode ser feita partição a partição, sem bloquear o mês corrente. Terceiro, a retenção regulatória fica mais simples de operar quando o dado é separado por mês.

## Índice clusterizado em (NotebookId, EntryId)

O índice clusterizado de `dbo.NotebookEntry` é sobre (NotebookId, EntryId). A leitura típica de um cientista é "todas as entradas deste caderno" ou "esta entrada deste caderno", e ambas viram seek no índice clusterizado. A ordem das colunas importa: NotebookId primeiro, para agrupar as entradas de um caderno fisicamente.

## Interação entre partição e índice

Em SQL Server, a chave de partição precisa fazer parte das chaves de índices únicos alinhados à partição. Aqui o índice clusterizado é descrito por (NotebookId, EntryId), então é preciso conferir como `CreatedMonth` entra na definição física para que o alinhamento funcione. Antes de alterar qualquer índice, verificar o script atual do esquema e não presumir. Consultas sem filtro em `CreatedMonth` varrem todas as partições para achar um caderno, o que é aceitável, mas custa mais.

## Padrões de consulta esperados

- Entradas de um caderno, sem filtro de data: seek pelo índice clusterizado em todas as partições.
- Entradas de um caderno em um intervalo de meses: eliminação de partições mais seek.
- Auditoria por período, em vários cadernos: varredura limitada às partições do período.
- Busca de uma entrada por identificador, quando o caderno é conhecido: seek direto.

## Consultas que dão problema

Buscar uma entrada só por EntryId, sem NotebookId, não aproveita o prefixo do índice clusterizado. Se esse caso aparecer no código C#, passar o NotebookId junto ou criar um índice não clusterizado específico, avaliando o custo de escrita.

## Escrita e fragmentação

Como o índice começa por NotebookId e não por tempo, inserções em cadernos diferentes caem em pontos diferentes do índice. Isso pode causar divisão de páginas. A partição por mês ajuda a limitar o estrago a partições recentes, e o fator de preenchimento pode ser ajustado nelas. Monitorar fragmentação das partições correntes.

## Trilha de auditoria

A trilha de auditoria não deve depender de updates silenciosos nas linhas de `dbo.NotebookEntry`. Mudanças relevantes precisam gerar registro de auditoria próprio. Não reaproveitar `CreatedMonth` como marca de modificação; ela serve só ao particionamento.

## Integração com RabbitMQ e instrumentos

A saída dos instrumentos chega por mensagens no RabbitMQ e é gravada como entrada ou anexo. O consumidor deve calcular `CreatedMonth` de forma determinística a partir do momento de criação da entrada, e não do momento em que a mensagem foi processada, para que reprocessamentos e reentregas caiam na mesma partição.

## Fuso horário

Decidir o mês em um único fuso, de preferência UTC, e manter isso em todos os pontos que calculam `CreatedMonth`. Divergência de fuso faz entradas perto da virada do mês caírem em partições diferentes conforme o serviço que gravou.

## Gestão de partições novas

É preciso garantir que exista partição para o mês seguinte antes da virada. Se a função de partição não cobrir o mês, as linhas caem na última partição e o esquema perde o benefício. Um job de manutenção deve criar partições com antecedência e alertar se faltar.

## Arquivamento e retenção

Partições antigas podem ser movidas ou comprimidas conforme a política de retenção. Nada disso apaga dado sujeito a auditoria sem aprovação da conformidade. Qualquer troca de partição para tabela de arquivo precisa preservar as referências ao Azure Blob Storage.

## Riscos e cuidados ao mudar

- Mudar a chave de partição exige reconstruir a tabela inteira; tratar como migração planejada.
- Mudar a ordem das colunas do índice clusterizado altera todos os planos de consulta por caderno.
- Mudar o tipo ou a semântica de `CreatedMonth` quebra o cálculo nos consumidores.

## Pendências

Documentar o script exato de criação da função e do esquema de partição, e confirmar como `CreatedMonth` entra nos índices únicos. Medir fragmentação real nas partições correntes depois de algumas semanas de uso.
