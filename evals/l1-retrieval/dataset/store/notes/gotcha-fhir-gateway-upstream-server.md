---
id: 01M0PHYAK8KZX1F3YHNT73DC2S
created: 2026-08-23T02:36-03:00
---

# fhir-gateway: reserva sem profissional volta como HTTP 422

Armadilha do fhir-gateway: quando uma reserva chega sem profissional, o servidor FHIR de destino recusa o Appointment. A resposta é `HTTP 422` com um OperationOutcome cujo texto é `Appointment.participant is required`. O erro aparece no fhir-gateway, mas a causa quase sempre está antes, na tela de agendamento ou no job que monta o recurso. Este registro serve para quem vir esse erro de novo e não quiser repetir a investigação.

## Sintoma

A recepção tenta confirmar um horário e a reserva fica pendente ou volta com falha. Nos logs do fhir-gateway aparece a resposta `HTTP 422` do servidor upstream. No corpo vem o OperationOutcome com a mensagem `Appointment.participant is required`. O texto é sempre o mesmo e não menciona qual reserva falhou, então é preciso cruzar com o identificador interno da tentativa.

Não é erro de rede nem de autenticação. O upstream está de pé, entendeu o pedido e rejeitou o conteúdo. Por isso repetir o envio não resolve: o mesmo payload gera o mesmo `HTTP 422` toda vez.

## Causa

O recurso Appointment em FHIR exige ao menos um participante. Na prática, para nós, o participante é o profissional que atende. Se a reserva não tem profissional associado, o fhir-gateway monta um Appointment sem participantes e o upstream devolve `Appointment.participant is required`.

O ClinicSlotter permite criar uma reserva em estado intermediário, por exemplo quando a recepção escolhe só sala e horário. Esse estado é válido para nós. Para o upstream não é.

## Como reproduzir

Crie uma reserva no ClinicSlotter sem escolher profissional e deixe o fluxo de sincronização rodar. Com um servidor FHIR de teste que valide o recurso, o resultado é o `HTTP 422` descrito acima. Se o servidor de teste for permissivo, ele aceita o recurso e o problema só aparece em produção. Isso já enganou gente: o teste passou e o ambiente real recusou.

## O que o fhir-gateway faz hoje

O fhir-gateway não valida a presença do profissional antes de enviar. Ele monta o recurso com o que recebe e repassa. A rejeição vem só do upstream, depois da viagem de ida e volta. O erro é registrado, mas ninguém é avisado de forma ativa.

O envio passa por um job do Sidekiq. Quando o job falha, a política de nova tentativa pode reenfileirá-lo, e cada tentativa gasta tempo e gera ruído no log sem chance de sucesso.

## Armadilha das novas tentativas

Um erro de validação não é transitório. Se o job tratar o `HTTP 422` como falha qualquer, ele entra em ciclo de retentativas e enche a fila. O certo é tratar esse caso como falha permanente: registrar, marcar a reserva e parar. Vale conferir a configuração de retentativas antes de supor que o problema é o upstream instável.

## Como diagnosticar

Primeiro, confirme o texto `Appointment.participant is required` no corpo da resposta. Se for outra mensagem do OperationOutcome, é outro problema. Depois abra a reserva no ClinicSlotter e veja se há profissional associado. Se não houver, achamos a causa. Se houver, procure um vínculo que não virou participante no recurso, por exemplo um profissional removido ou inativo no meio do caminho.

## Correção recomendada

Barrar antes do envio. O fhir-gateway deve recusar localmente qualquer reserva sem profissional, com mensagem clara em português para a recepção, em vez de depender do upstream. Assim a falha aparece cedo, sem tráfego inútil e sem retentativas.

No lado do Rails, a reserva só deve ser considerada pronta para sincronizar quando tiver profissional. Reservas incompletas ficam salvas, mas fora da fila de envio.

## O que não fazer

Não preencha um profissional genérico ou falso só para passar na validação. Isso suja o histórico clínico no sistema externo e pode atribuir consulta à pessoa errada. Também não ignore o erro em silêncio: a recepção acha que o horário está confirmado e o paciente aparece sem reserva do outro lado.

## Impacto para a recepção

O pessoal do balcão de clínicas pequenas trabalha rápido e muitas vezes reserva o horário antes de decidir quem atende. O sistema precisa deixar claro que a reserva ainda não foi sincronizada. Um aviso visível na lista de agendamentos evita que alguém diga ao paciente que está tudo confirmado.

## Testes a manter

Ter um teste que monte uma reserva sem profissional e verifique que o fhir-gateway não chama o upstream. Ter outro que simule a resposta `HTTP 422` com `Appointment.participant is required` e confirme que o job não é reenfileirado. Os dois cobrem as pontas do problema: a prevenção e o tratamento quando ela falha.

## Ambiente

O comportamento vale para qualquer ambiente que aponte para um servidor FHIR com validação ativa. No Heroku, o sintoma aparece nos logs da aplicação e dos workers. Em desenvolvimento local, depende do servidor usado; não confie na ausência de erro lá.

## Pontos em aberto

Ainda falta decidir se a validação local fica no fhir-gateway, no modelo da reserva, ou nos dois. A recomendação é nos dois: o modelo impede o estado inválido de entrar na fila, e o fhir-gateway é a última defesa. Também falta definir como a recepção é notificada quando uma reserva fica retida por falta de profissional.

## Resumo rápido

Se vir `HTTP 422` junto de `Appointment.participant is required`, a reserva não tem profissional. Não adianta repetir o envio. Corrija a reserva, depois sincronize.
