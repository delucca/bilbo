---
id: 01M0H73CGVKZQ9J3YYG1485Q2D
created: 2026-08-21T00:51-03:00
---

# Armadilha: início sobreposto na appointments-table

Inserir um agendamento com horário de início sobreposto na `appointments-table` falha com o erro `Mysql2::Error: Duplicate entry '12-2026-03-04 09:00:00' for key 'uniq_clinician_start'`. A falha acontece no banco, na hora do INSERT, e chega ao Rails como uma exceção de registro duplicado. Quem escreve código que cria agendamentos precisa tratar esse caso.

## O que acontece

A `appointments-table` tem um índice único chamado `uniq_clinician_start`. Ele cobre o clínico e o horário de início. Se já existe uma linha para o mesmo clínico com o mesmo início, o MySQL rejeita a nova linha. No exemplo da mensagem, o valor repetido é a combinação do clínico de id 12 com o início em 2026-03-04 às 09:00:00.

A mensagem de erro mostra o valor duplicado no formato `<clínico>-<data hora>`. Dá para ler dela qual clínico e qual horário colidiram, sem consultar a tabela.

## Como reconhecer

O sintoma na tela é um erro genérico ao salvar o agendamento. Nos logs aparece o texto completo do erro, que é `Mysql2::Error: Duplicate entry '12-2026-03-04 09:00:00' for key 'uniq_clinician_start'`. No Rails, ele costuma vir embrulhado em `ActiveRecord::RecordNotUnique`. Ao procurar nos logs, busque pelo nome do índice, `uniq_clinician_start`, que é o trecho mais estável da mensagem.

O erro também pode aparecer dentro de jobs do Sidekiq que criam agendamentos em lote ou a partir de recursos FHIR recebidos. Nesses casos o job falha e entra na fila de retentativas.

## Onde costuma aparecer

- Duplo clique no botão de confirmar da recepção, que dispara duas requisições para o mesmo horário.
- Dois funcionários da recepção reservando o mesmo clínico no mesmo horário quase ao mesmo tempo.
- Jobs do Sidekiq repetidos depois de uma falha parcial, que tentam criar de novo um agendamento já criado.
- Importações de agenda vindas de integração HL7 FHIR com o mesmo recurso enviado mais de uma vez.
- Testes que criam vários agendamentos com o mesmo clínico e o mesmo horário padrão.

## O que não resolve

A validação de unicidade do modelo Rails não basta sozinha. Ela faz uma consulta antes de salvar, e duas requisições simultâneas podem passar pela validação e só então colidir no banco. Por isso o erro do MySQL ainda aparece mesmo com a validação ligada.

Também não adianta apagar o índice para fazer o erro sumir. O índice faz parte do comportamento esperado da `appointments-table`, e outras partes do sistema contam com ele.

## Como tratar no código

Capture a exceção de registro duplicado no ponto em que o agendamento é criado e devolva ao usuário uma mensagem clara, dizendo que o clínico já tem um agendamento naquele horário. Na recepção, mostre os horários livres mais próximos do mesmo clínico.

Nos jobs do Sidekiq, trate a duplicata como um caso já resolvido quando o agendamento existente for o mesmo que o job tentava criar. Assim o job termina sem erro e não fica reentrando na fila de retentativas. Quando o agendamento existente for diferente, registre o conflito e não tente de novo.

## Como investigar um caso

Primeiro, pegue o clínico e o horário da mensagem de erro. Depois consulte a `appointments-table` pelo mesmo clínico e pelo mesmo início para ver a linha que já ocupa o horário. Verifique se ela está ativa ou cancelada, quem a criou e quando. Se for uma linha cancelada que ainda ocupa o horário, esse é um caso à parte e deve ser tratado como bug do fluxo de cancelamento.

Em produção no Heroku, faça essa consulta em modo somente leitura. Não corrija linhas à mão sem avisar a equipe da clínica afetada.

## Testes

Um teste de regressão deve criar dois agendamentos para o mesmo clínico com o mesmo início e confirmar que o segundo é recusado de forma tratada, sem exceção solta. Vale ter também um teste de concorrência simples, com duas criações em paralelo, para garantir que só uma vença e a outra receba a resposta amigável.

## Pendências

- Conferir se todos os pontos de criação de agendamento tratam a exceção, incluindo os jobs e a entrada FHIR.
- Revisar a mensagem mostrada à recepção para que cite o clínico e o horário em linguagem comum.
- Registrar em um lugar único o tratamento do erro `uniq_clinician_start`, para que novos fluxos reaproveitem o mesmo código.
