---
id: 01K98ANX9076F1P7T8HNGW0P8W
created: 2025-11-04T17:59-03:00
---

# Experimento do reminder-worker: o que apareceu

Anotação rápida sobre o que o experimento com o `reminder-worker` mostrou. Não é conclusiva, é o que lembro de ter visto enquanto mexia no Sidekiq. Vale refazer algumas medições antes de decidir qualquer coisa grande.

## Contexto

O `reminder-worker` roda no Sidekiq, dentro do app Rails, e envia lembretes de consulta para pacientes das clínicas pequenas que usam o ClinicSlotter. A ideia do experimento era entender por que alguns lembretes saíam tarde ou duplicados quando a recepção remarcava um horário perto da hora do envio.

O ambiente foi o de staging no Heroku, com o MySQL compartilhado. Não usei dados reais, só agendamentos de teste gerados a partir de disponibilidade de clínicos e salas.

## O que o experimento mostrou

- Quando um agendamento é remarcado, o job antigo continua na fila e dispara mesmo assim. O lembrete sai com o horário velho.
- O job lê o agendamento só na hora da execução, mas o texto do lembrete às vezes vinha de um dado já carregado antes. Isso explica parte dos horários errados.
- Com a fila cheia, o atraso ficou acima do que a recepção aceitaria. Não é problema do worker em si, é a fila compartilhada com outros jobs mais pesados.
- As retentativas padrão do Sidekiq geraram duplicatas quando a falha acontecia depois do envio e antes de marcar como enviado.

## Hipóteses ainda abertas

1. Separar o `reminder-worker` em uma fila própria, com concorrência menor que a usual, deve reduzir o atraso. Não medi isso direito.
2. Guardar uma marca de "enviado" antes da chamada externa, e não depois, evita duplicata mas pode perder um lembrete se o processo cair. Precisa decidir qual risco é pior.
3. A integração via HL7 FHIR não pareceu ser parte do problema, mas só olhei por cima. O recurso de agendamento chegava consistente nos testes.

## Exemplo de verificação no job

Ideia do guarda que testei, em Ruby, reconferindo o estado antes de enviar:

```ruby
def perform(appointment_id)
  appointment = Appointment.find(appointment_id)
  return if appointment.cancelled? || appointment.reminder_sent?

  ReminderMailer.with(appointment: appointment).deliver_now
  appointment.update!(reminder_sent: true)
end
```

Isso resolve o caso do horário velho, porque o job usa o registro atual. Não resolve a janela entre o envio e o `update!`.

## Próximos passos

- Repetir a medição de atraso com a fila separada, usando o limite de concorrência configurado hoje como base de comparação.
- Decidir entre idempotência por marca prévia ou por chave de deduplicação no envio.
- Ao remarcar, cancelar ou invalidar o job agendado, em vez de depender só da checagem na execução.
- Conferir se o fuso das clínicas entra em algum dos casos de horário errado. Suspeito que sim em um deles, mas não confirmei.
