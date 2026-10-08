---
id: 01M37YNYCAX74ERPMY7XZC23WK
created: 2026-09-23T17:17-03:00
---

# Especificação da ledger-entries-table: retenção

Esta nota substitui a nota anterior sobre "ledger entries rows table". O valor novo de retenção: as linhas da `ledger-entries-table` devem ser retidas por `24 months` antes de as partições serem destacadas. Isso substitui a retenção anterior.

## Regra de retenção

As linhas da `ledger-entries-table` ficam retidas por `24 months`. Só depois desse prazo as partições podem ser destacadas. A retenção anterior não vale mais; quem ler só esta nota deve usar `24 months`.

## O que mudou

A retenção antiga era mais curta e foi trocada. O valor atual é `24 months`. Não existe período de transição descrito aqui.

## Escopo

A regra vale para todas as linhas da `ledger-entries-table`, sem exceção por processadora de cartão ou por marketplace.

## Partições

O particionamento continua como estava. A mudança é só em quando destacar: depois de `24 months` de retenção, e não antes.

## Destacar partições

Destacar uma partição é a ação que só pode ocorrer depois do prazo. Antes disso a partição fica anexada e consultável.

## Conciliação

A conciliação dos arquivos de liquidação contra os lançamentos internos depende de histórico. Com `24 months` de retenção há mais janela para revisar divergências antigas.

## Revisão de divergências

Itens sinalizados para revisão podem referenciar lançamentos mais antigos. Com a retenção de `24 months`, esses lançamentos continuam presentes na `ledger-entries-table`.

## Impacto em armazenamento

Reter por mais tempo aumenta o volume em disco no PostgreSQL. Ainda falta medir o aumento real; vale planejar capacidade.

## Terraform

Se o dimensionamento do banco é definido no Terraform, rever esse dimensionamento por causa do prazo de `24 months`.

## Serviços em Go

Rotinas em Go que destacam ou removem partições devem respeitar `24 months`. Conferir se algum job ainda usa o prazo antigo.

## Kafka e gRPC

Consumidores de Kafka e chamadas gRPC que leem a `ledger-entries-table` não mudam de contrato. Só muda quanto histórico existe.

## Testes

Testar que uma partição dentro de `24 months` não é destacada e que uma fora do prazo pode ser.

## Pendências

- Verificar jobs de manutenção com o prazo antigo.
- Estimar o crescimento de armazenamento.
- Avisar a equipe de finance operations.

## Referência

Valor vigente: `24 months` para a `ledger-entries-table`. Esta nota é a fonte; a nota antiga fica obsoleta.
