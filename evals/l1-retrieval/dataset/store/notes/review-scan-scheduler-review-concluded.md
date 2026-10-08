---
id: 01JW7MPPYDGKKQDMGJD25YZ819
created: 2025-05-26T22:11-03:00
---

# Revisão do scan-scheduler: monitoramento de execuções puladas

Revisão do scan-scheduler feita com foco em observabilidade. Conclusão principal: o scan-scheduler não tem monitoramento para execuções puladas (skipped runs). Se uma varredura agendada não roda, ninguém é avisado. A recomendação da revisão é criar um alarme no CloudWatch chamado `MissedScanRun`, que dispara quando uma execução esperada do scan-scheduler não acontece. Este documento guarda o raciocínio para a próxima sessão não refazer a análise.

## Resultado da revisão

O scan-scheduler dispara as varreduras de configuração de infraestrutura que alimentam o AuditMesh. Hoje só se percebe uma execução perdida de forma indireta: alguém nota que os tickets de remediação no Jira pararam de aparecer ou que os dados no DynamoDB estão velhos. Isso é lento e depende de atenção humana. A revisão concluiu que falta monitoramento de execuções puladas e recomendou o alarme `MissedScanRun` no CloudWatch.

## Por que isso importa

Equipes de segurança em nuvem usam o AuditMesh para saber quais recursos violam política. Uma varredura pulada deixa uma janela cega: violações novas, avaliadas pelo Open Policy Agent, não viram ticket. O pior caso é silencioso, porque a ausência de tickets parece "tudo certo". Um alarme explícito separa "sem violações" de "sem varredura".

## Recomendação: alarme MissedScanRun

Criar o alarme `MissedScanRun` no CloudWatch para o scan-scheduler. A ideia é comparar o que era esperado com o que ocorreu: se não houver registro de execução concluída dentro do intervalo esperado, o alarme entra em estado de alarme. Pontos de desenho:

- Dado ausente deve contar como violação, não como estado normal (tratar dados faltantes como problema).
- O intervalo de avaliação deve ter folga para atrasos pequenos, evitando falso positivo.
- A notificação vai para o canal da equipe de plataforma de segurança.

## Como detectar uma execução pulada

Há duas fontes possíveis de sinal. A primeira é uma métrica emitida pela própria função Lambda ao terminar uma varredura com sucesso. A segunda é a métrica de invocações da função, mas ela só mostra que a função foi chamada, não que a varredura terminou. A preferência é pela métrica própria de conclusão, porque cobre também o caso em que a função roda e falha no meio.

## O que não foi coberto

A revisão não avaliou o desempenho das varreduras nem a lógica das políticas. Também não mediu quanto tempo as falhas ficaram sem ser notadas no passado. Esses pontos ficam fora deste registro.

## Riscos e dúvidas

- Falso positivo em janelas de manutenção ou quando o agendamento é pausado de propósito.
- Sobreposição com alarmes de erro que já existam para a função Lambda; convém não duplicar avisos.
- Decidir quem atende o alarme e em que prazo.

## Próximos passos

1. Definir a métrica de conclusão de varredura no scan-scheduler.
2. Criar o alarme `MissedScanRun` e testar com uma execução forçada a falhar.
3. Documentar o procedimento de resposta quando o alarme disparar.
4. Revisar depois de algumas semanas se o limiar gerou ruído.

## Resumo rápido

O scan-scheduler não monitora execuções puladas. A revisão recomenda o alarme `MissedScanRun` no CloudWatch como correção. Nada foi implementado ainda; é só a recomendação e o desenho inicial.
