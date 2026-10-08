---
id: 01KZSY7CKKHW86SN81XCQDDW7S
created: 2026-08-11T23:53-03:00
---

# Especificação da ledger-entries-table

Esta nota registra a especificação da `ledger-entries-table` no Ledgerlark: o que a tabela guarda, como é particionada e, principalmente, por quanto tempo as linhas ficam retidas. A regra de retenção é a parte que mais gera dúvida, então está logo abaixo e repetida na seção de operação.

## Regra de retenção

As linhas da `ledger-entries-table` devem ser retidas por `18 months` antes de as partições serem destacadas (detached). Ou seja: nenhuma partição pode ser destacada enquanto ainda tiver linhas mais novas que `18 months`. Só depois desse prazo a partição pode sair da tabela principal.

Destacar não é o mesmo que apagar. A decisão de arquivar ou descartar a partição já destacada é outro assunto e não está definida aqui. Se alguém precisar dessa decisão, deve abrir uma nota própria.

## O que a tabela guarda

A `ledger-entries-table` é a tabela no PostgreSQL com os lançamentos internos do razão. É contra ela que o Ledgerlark compara os arquivos de liquidação (settlement) das processadoras de cartão. Cada linha representa um lançamento contábil do marketplace, com valor, moeda, referência da transação e data de lançamento.

- Os lançamentos são tratados como append-only. Correção é feita com novo lançamento de estorno, não com update.
- A referência da transação é a chave usada na conciliação com o arquivo da processadora.
- A data de lançamento é a coluna que define a partição e também o início da contagem de retenção.

## Particionamento

A tabela é particionada por intervalo de data de lançamento, com partições de tamanho fixo (mensais na prática, mas o tamanho exato não faz parte desta especificação). O motivo do particionamento é justamente poder aplicar a retenção sem varrer a tabela inteira: destacar uma partição é uma operação de metadados, barata, e não um delete em massa.

Regras para quem mexer nisso:

- As partições futuras precisam existir com antecedência, senão o insert falha na hora do lançamento.
- A contagem de retenção usa a data de lançamento da linha mais recente da partição, não a data de criação da partição.
- Mudanças no esquema de partições passam por Terraform quando envolverem infraestrutura, e por migração SQL quando forem só da tabela.

## Operação e impacto na conciliação

O processo de conciliação em Go lê a `ledger-entries-table` e publica no Kafka os mismatches que vão para revisão. Os serviços conversam entre si por gRPC. Nada disso deve depender de linhas mais antigas que o prazo de `18 months`: se um arquivo de liquidação chegar com atraso e referenciar um lançamento já em partição destacada, a conciliação não vai encontrar a linha e vai marcar como mismatch.

Por isso:

- Não destacar partição antes de cumprir os `18 months`, mesmo que o disco esteja apertado.
- Antes de destacar, conferir que não há mismatch aberto apontando para linhas daquela partição.
- Se a retenção precisar mudar, atualizar esta nota primeiro e avisar o time de finance operations, que usa o histórico para auditoria.

## Pontos em aberto

- Quem executa o destacamento: job agendado ou procedimento manual. Hoje não está decidido nesta nota.
- Destino das partições destacadas (arquivamento em armazenamento frio ou descarte).
- Se existe exigência regulatória que obrigue prazo maior para algum tipo de lançamento. Se existir, ela prevalece sobre a regra de retenção acima e esta nota deve ser corrigida.

Enquanto esses pontos não forem fechados, vale apenas a regra de retenção: `18 months` antes de destacar qualquer partição da `ledger-entries-table`.
