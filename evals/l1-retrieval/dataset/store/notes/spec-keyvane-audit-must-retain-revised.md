---
id: 01M1SB8DXDN3NRMJW1C397NW0Z
created: 2026-09-05T14:53-03:00
---

# Retenção de eventos do keyvane-audit-log

Esta nota substitui a nota anterior sobre "keyvane audit must retain". O valor novo é este: o `keyvane-audit-log` deve reter eventos de auditoria por `365 days`, e esse prazo substitui o período de retenção anterior. Quem ler só esta nota já tem o que precisa: o prazo vigente é `365 days`, contado a partir do momento em que o evento é gravado, e o valor antigo não vale mais em lugar nenhum.

Escrevi isto com pressa, para que a próxima sessão não tenha de reconstruir a decisão. O texto é uma especificação: diz o que o componente deve fazer, não como está implementado hoje. Onde eu não sei o estado atual do código, digo que não sei. Detalhes que dependem de configuração de cada ambiente ficaram gerais de propósito, para a nota não envelhecer quando alguém mudar um nome de arquivo ou um parâmetro.

## Requisito de retenção

O requisito é um só e é simples. Todo evento de auditoria que o `keyvane-audit-log` recebe precisa continuar consultável e íntegro por `365 days`. Passado esse prazo, o evento pode ser removido, e não é obrigatório mantê-lo. Antes do prazo, nenhum processo do sistema pode apagar, truncar ou compactar o evento de modo que ele deixe de poder ser lido.

O prazo vale para todos os tipos de evento que o componente registra. Isso inclui emissão de credenciais de curta duração, rotação de segredos, leituras e escritas administrativas, falhas de autenticação mTLS, recusas de autorização e mudanças de política. Não existe prazo menor para eventos considerados de baixo valor. Se alguém quiser diferenciar por tipo no futuro, isso é outra decisão e deve virar outra nota, não uma exceção escondida nesta.

A contagem começa no instante em que o evento é gravado de forma durável, e não no instante em que o serviço de origem o produziu. Na prática a diferença é pequena, mas importa quando há fila ou reenvio: um evento reenviado mantém o horário original do fato e ganha o prazo a partir da gravação durável. Para auditoria, o horário do fato continua sendo o que aparece no registro; o prazo de retenção é uma propriedade do armazenamento.

O valor antigo foi trocado, não somado. Não há duas janelas, uma curta e uma longa. Existe apenas `365 days`. Se encontrar em documentação, painel, alerta ou configuração qualquer referência ao período anterior, trate como resíduo a corrigir. O período anterior não deve ser citado como alternativa válida.

O prazo é um mínimo garantido, e não um máximo obrigatório. O componente pode manter eventos além de `365 days` se a limpeza ainda não rodou, e isso não é defeito. O que é defeito é perder evento antes do prazo. Quando houver dúvida entre apagar cedo demais e apagar tarde demais, a escolha segura é apagar tarde.

## Motivo e contexto

O Keyvane emite credenciais de curta duração para serviços e rotaciona segredos sem redeploy. Quem usa são engenheiros de segurança e equipes de aplicação. Como as credenciais duram pouco, o rastro de auditoria é a única forma confiável de reconstruir depois quem pediu o quê, para qual identidade e com qual resultado. O segredo em si some rápido; o registro do que aconteceu precisa durar mais que ele.

A janela anterior era curta demais para investigações reais. Incidentes de segurança costumam ser descobertos muito depois do fato, às vezes por uma auditoria externa ou por um cliente. Quando a investigação chega, os eventos relevantes já tinham saído da retenção. Estender para `365 days` cobre ao menos um ciclo anual completo de revisão, que é o que as equipes de segurança pediram.

Há também o lado de conformidade. Uma retenção de um ano é um número que costuma aparecer em controles de auditoria, e ter o valor escrito e verificável evita discussão a cada revisão. Não estou citando uma norma específica aqui porque não li nenhuma nesta sessão; a decisão vem do pedido das equipes e da necessidade operacional, não de um artigo de lei que eu possa apontar.

O custo é armazenamento e tempo de consulta. Reter mais significa mais volume, e volume maior pesa em índices, em backup e em restauração. Aceitamos esse custo. A consequência é que o dimensionamento do armazenamento precisa ser revisto com o novo prazo, e não com o antigo. Esse é o ponto que mais facilmente fica para trás, então está nas pendências.

O componente conversa com o Vault da HashiCorp e usa o etcd para estado de coordenação. Identidades de serviço seguem SPIFFE e a comunicação usa mTLS. Os eventos de auditoria carregam a identidade que fez a chamada, e por isso o registro é sensível: quem lê o log vê quem acessa o quê. A retenção maior amplia a janela em que esses dados existem, e portanto amplia também a exigência de controle de acesso ao log.

## Comportamento esperado do keyvane-audit-log

Primeiro, a gravação. O componente deve gravar o evento de forma durável antes de confirmar a operação que o gerou, quando a operação for sensível. Se não conseguir gravar o evento de auditoria, a operação sensível deve falhar em vez de seguir sem registro. Essa regra já era a intenção anterior e não muda com o novo prazo; só a registro aqui para ficar completa.

Segundo, a imutabilidade dentro da janela. Um evento dentro dos `365 days` não pode ser alterado depois de gravado. Correções entram como novos eventos que referenciam o anterior. Quem tem acesso administrativo ao armazenamento não deve conseguir reescrever histórico sem deixar rastro, e a própria tentativa de alterar ou apagar evento dentro do prazo deve gerar um evento.

Terceiro, a expiração. Quando um evento ultrapassa `365 days`, ele fica elegível para remoção. A remoção deve ser feita por um processo de limpeza explícito e observável, não por efeito colateral de outra rotina. Esse processo precisa registrar quantos eventos removeu e qual foi o corte de tempo usado, para que seja possível provar depois que nada dentro do prazo foi apagado.

Quarto, a consulta. Qualquer evento dentro da janela deve poder ser consultado por intervalo de tempo, por identidade, por tipo de operação e por resultado. A consulta não deve ficar visivelmente mais lenta só porque a janela cresceu; se ficar, o problema é de índice ou de particionamento, e não motivo para encurtar o prazo. Encurtar o prazo para resolver desempenho contraria esta especificação.

Quinto, o relógio. A idade do evento depende de relógio confiável. O componente deve usar uma fonte de tempo consistente entre réplicas e não deve apagar evento com base em um relógio local que possa estar adiantado. Se houver suspeita de desvio de relógio, a limpeza deve ser adiada, nunca antecipada. Isso segue a regra de que, na dúvida, apaga-se tarde.

Sexto, a migração do prazo antigo para o novo. Eventos que já existiam quando o prazo mudou e que ainda não tinham sido removidos passam a seguir `365 days` a partir da gravação original. Eventos que já tinham sido removidos sob o prazo antigo não podem ser recuperados, e não vamos fingir que podem. Quem for investigar um período antigo deve saber que pode haver uma lacuna anterior à mudança, e essa lacuna deve estar documentada onde as equipes de segurança olham.

Sétimo, falhas parciais. Se o armazenamento do log ficar indisponível, o componente deve acumular eventos em uma fila durável local, dentro de um limite, e reenviar quando o destino voltar. Se a fila encher, as operações sensíveis passam a falhar, conforme a regra de gravação. A retenção só conta depois da gravação no destino durável, de modo que um período longo de indisponibilidade não encurta a vida útil real do evento.

Oitavo, o acesso. Ler o log exige identidade autorizada, autenticada por mTLS com identidade SPIFFE. Quem lê deve ficar registrado como quem lê. Um prazo de retenção maior só é aceitável se o acesso continuar restrito; do contrário estamos apenas guardando por mais tempo um dado que muita gente enxerga.

## Verificação, operação e pendências

Como saber se o requisito está cumprido. Há três verificações que valem a pena, todas em termos gerais porque não sei os nomes exatos de parâmetros em cada ambiente. Uma: conferir que a configuração de retenção do componente e do armazenamento por trás dele dizem `365 days` e não o valor antigo. Duas: escolher um evento gravado há quase um ano e confirmar que ele ainda é lido. Três: olhar o registro da última execução da limpeza e confirmar que o corte de tempo usado corresponde ao prazo vigente.

Um teste automatizado vale mais que uma checagem manual. O ideal é um teste que grava eventos com horários forjados em torno da fronteira do prazo, roda a limpeza e verifica que o que está dentro da janela permanece e o que está fora é elegível. Esse teste deve falhar se alguém reduzir o prazo sem querer. Não sei se esse teste já existe; precisa ser conferido, e se não existir, criá-lo é a primeira pendência.

Alertas. Deve haver alerta para limpeza que removeu volume anormalmente grande de eventos, porque isso pode indicar corte de tempo errado. Deve haver alerta também para limpeza que parou de rodar, porque aí o armazenamento cresce sem controle. Os dois erros são opostos e os dois importam, mas o primeiro é o grave: perde-se evidência. Alerta de crescimento de armazenamento é útil, mas secundário.

Backup e restauração. Os backups do log precisam respeitar o mesmo prazo. Um backup antigo que contenha eventos já expirados é um caminho para manter dado além do que se pretendia, e o contrário também vale: um backup que não cobre a janela inteira não serve para restaurar o histórico exigido. Convém decidir se a retenção dos backups acompanha exatamente a do log ou se tem uma margem, e escrever a decisão.

O que fica em aberto. Primeiro, dimensionar o armazenamento para o novo prazo, com uma estimativa de volume por dia multiplicada pela janela. Segundo, revisar índices e particionamento para que a consulta continue rápida. Terceiro, atualizar a documentação voltada a equipes de aplicação e a engenheiros de segurança, removendo menções ao prazo antigo. Quarto, confirmar que o painel de operação mostra o prazo vigente. Quinto, decidir a política dos backups, como dito acima.

Relação com outras notas. Quando o Vault não sobe, o próprio fluxo de emissão e de auditoria fica comprometido, e isso muda o que se espera encontrar no log durante o incidente. O caso está em [[keyvane-vault-fails-start]]. Vale ler essa nota ao investigar lacunas no log que coincidam com indisponibilidade do Vault, para não confundir falha de gravação com falha de retenção.

Cuidados para quem for mexer nisto depois. Não reduza o prazo para ganhar espaço ou velocidade sem uma decisão explícita e uma nota nova que substitua esta. Não crie exceções por tipo de evento dentro do código sem registrar. Não trate o prazo como máximo. E quando mudar o valor de novo, atualize esta nota em vez de abrir outra, porque o assunto é o mesmo e duas notas com valores diferentes são pior do que nenhuma.

Resumo curto para quem só passou os olhos: o `keyvane-audit-log` retém eventos por `365 days`, valor que substitui o anterior; é mínimo garantido; apagar cedo é defeito, apagar tarde não; a limpeza é explícita e registrada; o acesso ao log continua restrito e auditado.

Fontes: esta nota resulta de uma decisão informada na sessão, não de leitura de código. Não abri arquivos do repositório nem executei comandos para escrevê-la. Quem retomar o assunto deve confirmar no código e na configuração reais o que está dito aqui sobre o estado atual, e corrigir esta nota onde a realidade divergir.
