---
id: 01JYKBM2B314P1C83V3A4CCS23
created: 2025-06-25T07:54-03:00
---

# Segundo lembrete no reminder-worker e taxa de faltas

Testamos se um segundo lembrete, enviado pelo reminder-worker 2 hours antes da consulta, reduz as faltas (no-show). Reduz. A taxa caiu de 11% para 8%. Esta nota guarda o resultado, como o teste foi montado e o que ainda não sabemos, para que ninguém precise refazer a conta ou repetir o experimento sem necessidade.

## Resultado

Com o segundo lembrete disparado pelo reminder-worker 2 hours antes do horário marcado, a taxa de no-show foi de 11% para 8%. O valor de 11% é o da situação anterior, em que o paciente recebia só o lembrete original. O valor de 8% é o do grupo que recebeu também o segundo lembrete.

Em termos práticos: de cada cem consultas marcadas, cerca de três pacientes a mais passaram a comparecer. Para clínicas pequenas, que é o público da recepção do ClinicSlotter, isso significa menos horários vagos no fim do dia e menos encaixes de última hora.

A diferença é de três pontos percentuais, ou seja, uma queda relativa de pouco mais de um quarto. Não é um efeito enorme, mas é consistente com o que se espera de um lembrete perto do horário: ele pega quem esqueceu ou quem ainda não tinha decidido ir.

## Como foi o experimento

O reminder-worker já era o job do Sidekiq que envia o lembrete inicial. Para o teste, ele passou a agendar um segundo envio relativo ao horário da consulta, e não relativo ao momento do agendamento. Essa diferença importa: o segundo lembrete é calculado a partir do horário da visita, então remarcações precisam cancelar e reagendar o envio.

Pacientes foram comparados entre o período sem o segundo lembrete e o período com ele. O conteúdo da mensagem foi o mesmo nos dois casos, só o momento do envio extra mudou. Os dados de consulta vieram do banco MySQL, e o status de comparecimento foi o que a recepção registrou no sistema.

Um esboço da configuração usada no teste, só com valores desta nota:

```ruby
# reminder-worker: segundo lembrete
SECOND_REMINDER_OFFSET = 2.hours
# resultado observado: no-show 11% -> 8%
```

## Limites da conclusão

Alguns pontos para não superestimar o resultado:

- Não foi um teste com sorteio perfeito entre pacientes; há chance de outros fatores do período terem pesado, como sazonalidade e mudanças na agenda das clínicas.
- O registro de comparecimento depende da recepção marcar o status corretamente. Faltas não marcadas distorcem a taxa para qualquer lado.
- Não separamos por tipo de consulta, faixa etária ou canal de envio. O ganho pode ser maior em alguns grupos e nulo em outros.
- Só testamos um horário para o segundo lembrete, 2 hours antes. Outros intervalos não foram comparados, então não dá para dizer que esse é o melhor.

## Implicações para o reminder-worker

Se o segundo lembrete virar comportamento padrão, o reminder-worker precisa tratar alguns casos que o envio único não exigia:

- Remarcação e cancelamento: o job agendado no Sidekiq deve ser descartado ou ajustado quando a consulta muda, senão o paciente recebe aviso de um horário antigo.
- Consultas marcadas em cima da hora: se o agendamento ocorrer a menos de 2 hours da visita, o segundo lembrete não faz sentido e deve ser pulado, para não mandar duas mensagens quase juntas.
- Dados FHIR: o status do compromisso vindo da integração HL7 FHIR deve ser respeitado; consulta cancelada externamente não gera lembrete.
- Carga: dobrar os envios aumenta a fila do Sidekiq. No Heroku, vale acompanhar o tamanho da fila e a memória do dyno de worker depois de ligar a mudança.

## Próximos passos

Primeiro, decidir se o segundo lembrete fica ligado para todas as clínicas ou só como opção por clínica. Segundo, repetir a medição depois de algumas semanas com a regra ativa, para ver se os 8% se mantêm fora das condições do teste. Terceiro, se houver interesse, testar outro intervalo e comparar com o resultado de 2 hours. Quarto, registrar no painel da recepção a taxa de no-show por semana, para que a comparação com os 11% originais seja fácil de refazer sem depender de consulta manual ao banco.
