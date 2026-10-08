---
id: 01M23ACF6RVEQVDH9E30XK6DD2
created: 2026-09-09T11:50-03:00
---

# Escolha de ferramentas para o ledger-matcher

Escolhemos manter o `ledger-matcher` como um serviço Go pequeno, que lê os arquivos de liquidação já normalizados, compara com os lançamentos do ledger no PostgreSQL e publica as divergências no Kafka. Esta nota registra por que ficamos com essa combinação e o que descartamos. Não tem valores nem versões de propósito; o que vale é a direção.

## Contexto

O `ledger-matcher` é a parte do Ledgerlark que faz o casamento entre o que o processador de cartão diz que liquidou e o que o ledger interno registrou. O resultado vai para a fila de revisão da operação financeira. Quem usa isso são times de finanças de marketplaces, então o que importa é que o casamento seja previsível e que dê para explicar cada divergência depois.

O debate na época foi entre fazer o casamento dentro do banco, com SQL pesado, ou fazer em Go, lendo em lotes e aplicando as regras no código. Ficamos com um meio-termo, descrito abaixo.

## Decisão

O banco guarda os dados e faz só o que ele faz bem: filtrar, juntar por chave e garantir unicidade. As regras de casamento (tolerância, agrupamento de parcelas, estornos) ficam em Go, em funções puras e testáveis. Nada de regra de negócio em procedure armazenada.

As divergências saem como eventos no Kafka, e não por chamada direta ao serviço de revisão. Assim a revisão pode atrasar ou cair sem travar o casamento.

A interface com outros serviços internos é gRPC. Para consultas pontuais ("por que esta linha não casou?") o `ledger-matcher` expõe um método de leitura, sem escrita.

## Por que não SQL pesado

Tentamos um protótipo com a lógica toda em consultas. Funcionava, mas cada ajuste de regra virava uma mudança difícil de revisar e de testar sem um banco populado. Em Go, o caso de teste é uma tabela de entradas e saídas esperadas, e o revisor lê a regra como código normal.

Também pesou o plano de execução: consultas grandes mudavam de comportamento conforme o volume, e isso dificultava prever o tempo de um fechamento.

## Por que Kafka para as divergências

- Reprocessar é simples: dá para reler o tópico e reconstruir a fila de revisão.
- O consumidor da revisão escala separado do casamento.
- Fica um registro ordenado do que foi emitido, útil em auditoria.

O custo é operar mais uma peça e lidar com entrega repetida. Por isso o consumidor precisa ser idempotente, e a chave do evento tem que identificar a divergência de forma estável.

## Infraestrutura

Terraform descreve o banco, os tópicos e o deploy do serviço. A regra é que nada do `ledger-matcher` seja criado à mão em ambiente nenhum. Se algo foi mexido no console, volta para o código antes de qualquer outra mudança.

Um trecho só para lembrar a ordem do fluxo:

```text
settlement file -> ledger-matcher -> kafka -> review queue
                        |
                    postgresql
```

## Pontos em aberto

- Ainda não decidimos como lidar com arquivos de liquidação reenviados pelo processador com correções. Hoje tratamos como arquivo novo, e isso pode gerar divergências duplicadas.
- A política de retenção dos tópicos precisa ser combinada com quem cuida de auditoria.
- Falta documentar quais regras de tolerância são configuráveis por marketplace e quais são fixas no código.

## Se for revisitar

Reabrir esta decisão só faz sentido se as regras de casamento ficarem tão simples que o SQL puro compense, ou se o Kafka virar um peso operacional maior que o ganho de desacoplamento. Antes disso, prefira melhorar os testes das regras em Go a mover lógica para o banco.
