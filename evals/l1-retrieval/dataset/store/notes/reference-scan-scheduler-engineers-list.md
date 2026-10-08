---
id: 01M0A2ZBWJG3WC4HFEVC18FQYF
created: 2026-08-18T06:24-03:00
sources:
  - "doc: AuditMesh Scheduler Runbook"
---

# scan-scheduler: referência de operação e listagem de regras

Nota de referência rápida sobre o `scan-scheduler`, o componente do AuditMesh que decide quando cada varredura de configuração de nuvem roda. Escrevi com pressa, para quem precisa achar as coisas sem reler o código. Onde não tenho certeza de um detalhe, deixo geral de propósito e não invento valor.

## Para que serve

O `scan-scheduler` dispara as varreduras do AuditMesh em horários definidos. Ele não avalia política nem abre ticket. Só garante que a varredura certa comece no momento certo, para o escopo certo. Quem avalia é o Open Policy Agent, e quem cria o ticket de remediação é a etapa seguinte, que fala com o Jira.

O uso principal é das equipes de segurança em nuvem. Elas esperam que as varreduras aconteçam de forma previsível, sem lacunas longas entre uma e outra. Se o `scan-scheduler` para de disparar, ninguém vê erro de política, só silêncio. Por isso vale saber olhar as regras dele.

## Como listar as regras

O jeito direto de ver quais regras do `scan-scheduler` existem na conta é listar as regras do EventBridge pelo prefixo que o projeto usa. Engenheiros listam as regras com o comando abaixo:

```bash
aws events list-rules --name-prefix auditmesh-
```

O comando mostra o nome de cada regra, o estado (habilitada ou desabilitada) e a expressão de agendamento. Rode na conta e na região onde o scheduler está implantado. Se a saída vier vazia, quase sempre o perfil ou a região da CLI está errado, e não é que as regras sumiram.

## Como ler a saída

O que importa na listagem:

- o estado da regra: uma regra desabilitada não dispara nada, e isso costuma ser a causa de varredura que "parou";
- a expressão de agendamento: pode ser uma taxa fixa ou uma expressão cron, e a hora é sempre em UTC;
- a descrição, quando existe, que deve dizer a qual escopo de varredura a regra pertence.

A listagem não mostra o alvo da regra. Para ver para onde ela aponta é preciso consultar os alvos separadamente. Normalmente o alvo é uma função Lambda.

## Arquitetura em resumo

O fluxo é simples. Uma regra do EventBridge dispara em um horário, aciona uma função Lambda escrita em Python, e essa função registra a intenção de varredura e inicia o trabalho. O estado das execuções fica no DynamoDB. A avaliação das políticas usa o Open Policy Agent, e o resultado alimenta a criação de tickets no Jira.

O `scan-scheduler` é só a primeira parte dessa cadeia. Quando algo não chega ao Jira, não presuma que o problema está nele. Confirme primeiro se a regra disparou.

## Regras e escopos

Cada regra representa uma frequência de varredura para um conjunto de recursos. Em geral há regras mais frequentes para o que muda rápido e regras mais espaçadas para o que muda pouco. A divisão exata mudou ao longo do tempo, então confira na listagem em vez de confiar em memória ou em documento antigo.

Quem adiciona um escopo novo deve criar a regra seguindo o mesmo padrão de nome das existentes. Assim a listagem por prefixo continua mostrando tudo. Uma regra criada fora do padrão fica invisível para esse comando e some das conferências de rotina.

## Estado no DynamoDB

O DynamoDB guarda o registro das execuções: o que foi pedido, o que está em andamento e o que terminou. O scheduler usa esse registro para não iniciar duas varreduras do mesmo escopo ao mesmo tempo. Se uma execução ficou presa em andamento, a próxima pode ser pulada de propósito, e isso se confunde com falha do agendador.

Ao investigar, compare o horário em que a regra disparou com o registro da execução correspondente. Se a regra disparou e não há registro, o problema está na função. Se há registro preso, o problema está na execução anterior.

## Função Lambda

A função é em Python e deve ser curta. Ela não faz a varredura pesada, só orquestra. Se começar a acumular lógica de avaliação, é sinal de que algo foi parar no lugar errado. Os logs dela vão para o CloudWatch, e é lá que se confirma se a invocação aconteceu e com qual entrada.

Cuidado com tempo limite e concorrência. Uma função que estoura o limite no meio do registro pode deixar estado inconsistente no DynamoDB, e isso reaparece depois como execução presa.

## Diagnóstico quando a varredura não rodou

Ordem que costumo seguir:

1. listar as regras com o comando da seção acima e conferir se a esperada existe e está habilitada;
2. conferir a expressão de agendamento e lembrar que é em UTC;
3. olhar os logs da função no horário esperado;
4. olhar o registro da execução no DynamoDB;
5. só então suspeitar do Open Policy Agent ou do Jira.

Pular direto para o passo cinco é o erro mais comum e faz perder tempo.

## Mudanças nas regras

Alterar uma regra mexe diretamente em quando a segurança enxerga os problemas. Faça mudança de agendamento com aviso para a equipe que consome os tickets, porque uma frequência maior gera mais tickets de uma vez e uma menor atrasa a detecção. Registre o motivo no próprio pedido de mudança.

Desabilitar uma regra para manutenção é aceitável, mas anote quem desabilitou e reabilite ao fim. Regra esquecida desabilitada é uma das causas mais frequentes de lacuna de cobertura.

## Armadilhas conhecidas

- A listagem depende da conta e da região da CLI. Perfil errado dá resultado vazio sem erro.
- Regra fora do padrão de nome não aparece na listagem por prefixo.
- Horário em UTC causa confusão quando se compara com o relógio local.
- Execução presa no DynamoDB faz o scheduler pular varreduras sem avisar.
- Ver a regra habilitada não prova que a função foi invocada com sucesso.

## O que este componente não faz

O `scan-scheduler` não decide quais políticas valem, não interpreta resultado de varredura e não cria nem fecha ticket. Pedidos sobre conteúdo de política vão para a parte do Open Policy Agent. Pedidos sobre formato ou duplicidade de ticket vão para a parte da integração com o Jira.

## Pendências desta nota

Falta registrar aqui a lista atual de escopos e suas frequências, e o procedimento exato para criar uma regra nova. Quando alguém confirmar esses pontos na conta, acrescentar nesta nota em vez de abrir outra sobre o mesmo componente.
