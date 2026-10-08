---
id: 01K1J9T36GQNXA7EX46SPV5MT2
created: 2025-08-01T04:51-03:00
---

# appointments-table: cuidados ao mexer

Anotação geral sobre o que pode dar problema ao mexer na appointments-table. É a tabela mais quente do ClinicSlotter: o agendamento do balcão, os jobs do Sidekiq e a troca de dados via FHIR passam por ela. Qualquer mudança aqui tem efeito em vários lugares ao mesmo tempo, então vale desacelerar um pouco antes de abrir a migration.

## Migrations e tamanho da tabela

A appointments-table cresce sem parar, porque o histórico fica lá. Uma alteração de coluna ou de índice que parece barata em desenvolvimento pode travar a tabela em produção no MySQL. Antes de rodar no Heroku, pense em como a mudança será aplicada: se ela reescreve a tabela inteira, se bloqueia escritas e por quanto tempo. Prefira mudanças em etapas pequenas: adicionar a coluna nula, preencher aos poucos, só depois impor restrição.

- Não misture mudança de esquema e backfill na mesma migration.
- Backfill em lotes, fora do horário de uso do balcão, de preferência por um job e não dentro da migration.
- Remover coluna é a última etapa. Primeiro o código para de ler e escrever, depois sai a coluna, em outro deploy. Lembre que o Rails guarda em cache a lista de colunas e processos antigos podem quebrar durante o deploy.
- Índice novo em tabela grande também custa. Confira se ele realmente serve às consultas da agenda antes de criar.

## Conflitos de horário e de sala

A regra de não sobrepor clinicianos e salas depende dos dados desta tabela. Se só a aplicação verifica a sobreposição, duas pessoas no balcão podem reservar o mesmo horário quase ao mesmo tempo. Ao mudar colunas de horário, duração, sala ou clínico, revise onde a checagem acontece e se há proteção no banco, como lock ou restrição, além da validação do model. Validação do Rails sozinha não segura concorrência.

Cuidado também com o que significa cada coluna de tempo: início, fim e duração podem estar redundantes. Mudar uma e esquecer a outra gera agenda inconsistente que ninguém percebe até o dia da consulta. Fuso horário é outra fonte de bug: confirme em que zona o valor é gravado e em que zona é exibido para cada clínica.

Cancelamento e remarcação normalmente não apagam a linha. Qualquer consulta nova precisa filtrar pelo estado correto, senão horários cancelados continuam bloqueando a agenda, ou, pior, horários ativos aparecem livres.

## Sidekiq, callbacks e FHIR

Existem callbacks e jobs que disparam quando uma consulta é criada ou alterada: lembretes, sincronização, notificações. Ao mudar a appointments-table, procure tudo que reage a ela antes de alterar nomes de colunas ou estados.

- Jobs enfileirados guardam argumentos antigos. Se mudar a assinatura de um job ou o formato de um valor, os que já estão na fila ainda vão rodar com o formato velho. Faça o código aceitar os dois por um tempo.
- Job que enfileira dentro de uma transação pode rodar antes do commit e não achar a linha. Use enfileiramento após o commit.
- Jobs podem rodar duas vezes. Tudo que toca a tabela precisa ser idempotente.
- O mapeamento para o recurso de consulta do HL7 FHIR depende de status e de campos desta tabela. Se um estado interno mudar de nome ou ganhar um valor novo, confira como ele é traduzido para o status FHIR, e o caminho inverso na importação. Um valor sem mapeamento costuma falhar em silêncio.
- Dados de paciente são sensíveis. Não coloque dados pessoais em logs, em mensagens de erro nem em argumentos de job só por conveniência.

## Testes e verificação

Os testes com dados pequenos não mostram problema de desempenho nem de lock. Para mudanças que mexem em consultas ou índices, olhe o plano de execução no MySQL com um volume parecido com o real. Consultas da tela da agenda são as mais sensíveis, porque o balcão as usa o dia inteiro.

Teste os casos de borda de agenda: consulta que termina exatamente quando outra começa, consulta que atravessa a meia-noite, mudança de horário de verão, remarcação para o mesmo horário, e duas escritas simultâneas no mesmo slot. Esses são os que quebram na prática.

Antes de mergulhar, tenha um plano de reversão. Migration que perde dados não volta com rollback; se for o caso, deixe isso escrito no PR e combine com quem opera o Heroku. Depois do deploy, vigie erros de jobs e reclamações do balcão por um tempo.
