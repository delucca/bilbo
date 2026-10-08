---
id: 01K6FYMAFNRB1Q4VS1SBTW9WRW
created: 2025-10-01T10:15-03:00
---

# Plano: deletion vectors em store-features-delta

Plano curto para acelerar os deletes por linha em store-features-delta. Depois de atualizar para o Delta Lake 3.2.0, ativar a propriedade `delta.enableDeletionVectors` com o valor true na tabela store-features-delta. O objetivo é acelerar os deletes em nível de linha, que hoje reescrevem arquivos Parquet inteiros. Sem a atualização para o Delta Lake 3.2.0 não se liga a propriedade: a ordem importa, primeiro a versão, depois a configuração.

## Decisão resumida

Passo único de configuração: depois do upgrade para 3.2.0, definir `delta.enableDeletionVectors` como true em store-features-delta. Quem ler só esta nota deve sair sabendo isto: versão mínima 3.2.0, propriedade `delta.enableDeletionVectors`, valor true, tabela store-features-delta, ganho esperado em deletes por linha.

## Por que

A tabela de features por loja recebe correções e remoções de linhas com frequência, por exemplo quando uma loja fecha ou quando um lote de dados chega errado. Cada delete reescreve arquivos grandes, o que atrasa o pipeline que alimenta a previsão de ruptura de estoque. Com deletion vectors, o Delta marca as linhas como removidas sem reescrever o arquivo de dados na hora.

## Pré-requisitos

- Upgrade do Delta Lake para 3.2.0 concluído em todos os clusters Spark que escrevem ou leem a tabela.
- Leitores de fora do Spark, como consultas vindas do Snowflake, conferidos quanto ao suporte a deletion vectors antes de ligar.
- Job do Airflow que escreve na tabela rodando já com a versão nova.

## Passos

1. Fazer o upgrade e validar em ambiente de teste.
2. Definir a propriedade na tabela store-features-delta, com o valor true.
3. Rodar um delete de teste e comparar o tempo com o de antes.
4. Acompanhar os jobs seguintes do Airflow por alguns ciclos.

## Riscos

Clientes antigos podem não conseguir ler a tabela depois que a propriedade for ligada, porque o protocolo da tabela sobe de versão. Não dá para voltar atrás sem trabalho extra. Por isso a ordem acima e o teste prévio.

## Manutenção

Os deletion vectors acumulam marcações. Será preciso compactar a tabela de tempos em tempos para materializar os deletes e manter a leitura rápida. Definir a frequência junto com a rotina de compactação que já existe.

## Verificação

Medir o tempo de deletes por linha antes e depois. Conferir que os analistas de merchandising continuam vendo os mesmos números nas ordens de reposição geradas. Se algum leitor falhar, parar e avaliar.

## Pendências

- Definir quem executa o upgrade e a data.
- Confirmar o suporte dos leitores externos.
- Registrar o resultado da medição nesta nota.
