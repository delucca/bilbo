---
id: 01K5VP0ZVX20VNQPE1VPA7Y6F3
created: 2025-09-23T13:20-03:00
---

# Resumo semanal do sales-ingest-pipeline

Semana corrida, anotei rápido o que mexeu no sales-ingest-pipeline e o que ficou pendente. Nada aqui fixa valor nem decisão, é só o estado geral do trabalho.

## Contexto da semana
Foco em estabilizar a ingestão de vendas das lojas antes de mexer em qualquer coisa nova. O resto da cadeia, previsão de ruptura e geração de pedidos, depende desses dados chegando completos.

## Leitura dos arquivos de venda
Revisei o job em Scala que lê os arquivos brutos das lojas. Achei trechos repetidos de parsing que valem ser unificados. Ainda não refatorei, só marquei os pontos.

## Schema e campos opcionais
Algumas redes mandam campos que outras não mandam. O tratamento de nulos está espalhado. Vale concentrar isso num único lugar e documentar o que é opcional.

## Escrita no Delta Lake
A escrita nas tabelas Delta funcionou sem surpresas esta semana. Olhei o particionamento e acho que ele merece uma revisão com calma, mas sem pressa.

## Arquivos pequenos
Continua a tendência de gerar muitos arquivos pequenos em dias de pouco movimento. A compactação ajuda, só que ainda roda em horário pouco previsível.

## Deduplicação
Reenvios de vendas pelas lojas continuam aparecendo. A deduplicação atual cobre o caso comum, mas conversei sobre casos de borda que ainda escapam.

## Dados atrasados
Lojas que enviam com atraso bagunçam a janela de processamento. Precisamos combinar com o time de previsão como tratar o que chega depois.

## Orquestração no Airflow
Ajustei dependências entre tarefas da DAG para a ordem ficar mais clara. Também revisei as políticas de nova tentativa, que estavam permissivas demais em alguns passos.

## Alertas
Os alertas de falha chegam, mas alguns são ruidosos. Quero separar o que exige ação imediata do que só informa.

## Carga para o Snowflake
A etapa de exportação para o Snowflake seguiu estável. Falta conferir se a reconciliação entre origem e destino está cobrindo todas as tabelas relevantes.

## Qualidade dos dados
Comecei a listar verificações básicas de qualidade: duplicatas, lojas sem dados, valores absurdos. Ainda é rascunho e não está no pipeline.

## Testes
Os testes de unidade do parsing estão fracos para formatos incomuns. Anotei casos reais para virarem testes.

## Desempenho
Sem mudança perceptível no tempo de execução. Suspeito de shuffles desnecessários numa agregação intermediária, preciso confirmar no plano de execução.

## Documentação
O passo a passo de como reprocessar um dia está desatualizado. Vou corrigir quando o fluxo de reprocesso estiver firme.

## Riscos
O maior risco é uma rede nova entrar com formato diferente e quebrar o parsing sem aviso. Falta uma validação de entrada mais cedo no fluxo.

## Próximos passos
Unificar o parsing, melhorar a deduplicação, calibrar os alertas e escrever os testes dos formatos incomuns.

## Pendências com outras pessoas
Preciso falar com quem cuida da previsão sobre dados atrasados e com os analistas de merchandising sobre quais lojas dão mais problema.
