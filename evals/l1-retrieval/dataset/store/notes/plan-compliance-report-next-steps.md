---
id: 01K0F5Q0C9ARSVEMNSHPGBFXC2
created: 2025-07-18T13:26-03:00
---

# Plano geral de próximos passos do compliance-report-api

Anotação rápida sobre o que fazer a seguir no compliance-report-api. Nada aqui fixa valores; são direções gerais para retomar o trabalho sem reaprender o contexto. O componente gera relatórios de conformidade a partir das entradas do caderno eletrônico e da trilha de auditoria, e é lido principalmente por pessoas da área de compliance.

## Objetivo geral

Deixar o compliance-report-api previsível, rastreável e fácil de operar. O relatório precisa refletir fielmente a trilha de auditoria, sem lacunas silenciosas. Quem consome o relatório deve poder confiar que o conteúdo bate com o que foi sincronizado dos instrumentos.

## Estado de partida

O serviço já expõe a geração de relatórios e consulta os dados de auditoria no SQL Server. Anexos e arquivos grandes ficam no Azure Blob Storage. Eventos de sincronização chegam pelo RabbitMQ. O que falta é mais acabamento do que estrutura nova: revisar os contratos, as bordas e a observabilidade.

## Revisar o contrato da API

Reler os endpoints de relatório e anotar o que está ambíguo. Conferir nomes de campos, formato de resposta e códigos de retorno para que sejam consistentes entre si. Separar o que é contrato público do que é detalhe interno, e documentar só o primeiro.

## Paginação e filtros

Relatórios grandes precisam de paginação estável e filtros previsíveis. Verificar se a ordenação é determinística, porque ordem instável quebra a confiança de quem audita. Alinhar os filtros por período, usuário e instrumento com o que a equipe de compliance realmente pede.

## Consistência com a trilha de auditoria

Garantir que o relatório nunca mostre dados que a trilha não sustente. Rever o ponto em que o relatório lê dados: se há leituras fora de uma visão consistente, corrigir. Registrar claramente quando uma entrada ainda está pendente de sincronização, em vez de omiti-la.

## Integridade e imutabilidade

Entradas auditadas não podem ser alteradas depois. Revisar se alguma rota de geração ou reprocessamento permite reescrever histórico. Se permitir, fechar essa porta antes de qualquer funcionalidade nova.

## Geração assíncrona

Relatórios pesados devem rodar fora do ciclo da requisição. Avaliar o uso de filas para disparar a geração e devolver um identificador de acompanhamento. Definir o comportamento em caso de falha parcial e de reentrega de mensagens.

## Idempotência

Mensagens do RabbitMQ podem chegar mais de uma vez. Conferir que processar duas vezes o mesmo pedido não gera relatório duplicado nem estado inconsistente. Pensar na chave de deduplicação junto com o time de sincronização.

## Armazenamento dos relatórios gerados

Decidir, em conversa com compliance, se o relatório gerado é guardado ou recalculado sob demanda. Se for guardado, tratar o Azure Blob Storage como destino, com política de retenção definida pela área responsável. Não assumir retenção por conta própria.

## Controle de acesso

Revisar quem pode pedir qual relatório. Cientistas e responsáveis de compliance têm necessidades diferentes, e o serviço deve aplicar isso no servidor, não confiar no cliente. Registrar na própria trilha cada geração e cada download de relatório.

## Desempenho das consultas

Olhar os planos de execução das consultas mais pesadas no SQL Server e ver onde faltam índices ou sobram leituras. Medir antes de mexer. Evitar carregar tudo em memória ao montar relatórios extensos; preferir leitura em fluxo.

## Tratamento de erros

Padronizar as respostas de erro e separar falha do cliente de falha do servidor. Mensagens não devem vazar detalhes internos. Falhas de dependência, como fila ou armazenamento fora do ar, precisam de tentativa controlada e de um estado visível.

## Observabilidade

Adicionar logs estruturados com correlação entre requisição, mensagem e relatório gerado. Criar métricas básicas de duração, fila e falha. Montar alertas só depois de entender o comportamento normal.

## Testes

Cobrir primeiro as regras de consistência com a auditoria, depois o contrato. Usar dados sintéticos que imitem saídas de instrumentos, incluindo casos estranhos: entradas atrasadas, duplicadas e fora de ordem. Incluir testes de integração com banco e fila reais em ambiente isolado.

## Documentação

Escrever uma página curta para quem opera o serviço e outra para quem consome o relatório. Manter o texto curto e atualizado junto com o código. Marcar o que ainda é incerto em vez de afirmar.

## Perguntas em aberto

Falta alinhar com compliance o formato final dos relatórios e o que conta como evidência aceitável. Falta saber se existem exigências externas que afetem retenção e assinatura. Essas respostas devem vir de quem responde pela regra, e não ser deduzidas do código.

## Ordem sugerida

Primeiro integridade e consistência, depois idempotência e erros, depois desempenho e observabilidade, por fim documentação e acabamento do contrato. Reavaliar a ordem se aparecer algum problema de conformidade real durante a revisão.
