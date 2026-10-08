---
id: 01KN5ZZJHF7ZC0D6T9DJ7TV1P6
created: 2026-04-01T23:22-03:00
---

# Resultado do teste de carga do config-scanner

Anotação rápida sobre o desempenho do `config-scanner` no teste de carga de setembro. O número principal: o `config-scanner` varreu `1,840 accounts` em `42 minutes`. Serve de linha de base para comparar as próximas medições.

## Resumo do resultado

No teste de carga de setembro, o `config-scanner` varreu `1,840 accounts` em `42 minutes`, de ponta a ponta, da leitura das configs até a geração dos tickets de remediação. Não houve ajuste fino específico para esse teste; foi a configuração normal do componente.

## Contexto

O `config-scanner` faz parte do AuditMesh. Ele lê configurações de infraestrutura em nuvem, avalia as regras escritas para o Open Policy Agent e entrega as violações encontradas. Depois disso, os tickets de remediação vão para o Jira. Roda em AWS Lambda, escrito em Python, e guarda estado no DynamoDB.

## Como ler o número

Os `42 minutes` são o tempo total do teste, não o tempo por conta. Dividindo, dá uma média de poucos segundos por conta, mas isso mistura contas pequenas e grandes. Não tratar a média como garantia para uma conta isolada.

## Resumo em código

```text
config-scanner, teste de carga de setembro
contas varridas: 1,840 accounts
duração total:   42 minutes
```

## O que ainda não sabemos

- Onde ficou o gargalo: avaliação das políticas no OPA, leitura das configs, escrita no DynamoDB ou criação dos tickets no Jira.
- Se o tempo cresce de forma linear com o número de contas.
- Quanto da duração veio de limites de concorrência do Lambda.
- Se houve limitação de taxa (throttling) no Jira ou no DynamoDB durante o teste.

## Riscos

Se a base de clientes crescer bastante, a mesma execução pode passar de um tempo aceitável para equipes de segurança. Vale confirmar o limite de duração das funções Lambda e como o trabalho é dividido entre invocações, para que uma varredura grande não seja cortada no meio.

## Próximos passos

1. Repetir o teste com a mesma massa de contas e registrar tempo por etapa.
2. Medir o uso do DynamoDB e as respostas de erro do Jira.
3. Comparar o resultado com esta linha de base antes de qualquer mudança de paralelismo.
4. Atualizar esta nota com os novos números, em vez de criar outra.

## Fonte dos dados

Os números vêm do relatório do teste de carga de setembro, repassado para esta nota. Não reproduzi o teste nesta sessão.
