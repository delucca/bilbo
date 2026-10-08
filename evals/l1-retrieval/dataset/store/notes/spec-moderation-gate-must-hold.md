---
id: 01KW3QZ2HSRHG55TF51VAXFV0V
created: 2026-06-27T02:13-03:00
---

# Especificação do moderation-gate

Este documento descreve o que o `moderation-gate` precisa fazer no TownHall Pulse e quais limites ele não pode violar. Foi escrito às pressas, a partir do que a equipe de produto e de operação de eventos pediu, então prefere ser direto a ser bonito. A regra central cabe em uma frase: o moderation-gate deve segurar toda mensagem sinalizada para revisão humana e deve chegar a uma decisão de aprovação automática em no máximo 2 seconds para as mensagens não sinalizadas. As duas metades dessa frase valem ao mesmo tempo. Nenhuma mensagem sinalizada pode passar sem que uma pessoa a tenha visto, e nenhuma mensagem limpa pode ficar esperando além do prazo de 2 seconds por uma decisão automática.

O público desta nota são quem mexe no backend em Elixir e Phoenix, quem mexe no cliente em Next.js e quem opera a plataforma durante eventos grandes. Quem só leu esta nota deve conseguir responder: o que o moderation-gate garante, o que ele segura, o que ele libera sozinho, quanto tempo ele tem para decidir, e o que acontece quando algo quebra.

## Objetivo e contrato

O TownHall Pulse roda enquetes ao vivo e perguntas e respostas em eventos virtuais grandes. Em um evento desse tipo, chegam muitas mensagens de participantes ao mesmo tempo, e o produtor do evento e o gerente de comunidade precisam de moderação em tempo real sem virar gargalo. O moderation-gate é o ponto por onde toda mensagem de participante passa antes de ser mostrada a outros participantes, ao palestrante ou ao painel do produtor. Ele é uma porta: ou a mensagem sai aprovada, ou fica retida, ou é rejeitada por uma pessoa.

O contrato tem duas obrigações explícitas.

A primeira é a retenção. Toda mensagem que o moderation-gate considera sinalizada fica retida para revisão humana. Não existe caminho em que uma mensagem sinalizada seja publicada por decisão automática. Isso vale mesmo quando a fila de revisão está grande, mesmo quando não há moderador online no momento e mesmo quando o evento está perto do fim. Se não houver ninguém para revisar, a mensagem continua retida. O custo de uma mensagem sinalizada que espera é aceitável; o custo de uma mensagem sinalizada que vaza não é.

A segunda é a latência da aprovação automática. Para mensagens não sinalizadas, o moderation-gate precisa chegar a uma decisão de aprovação automática dentro de 2 seconds. Esse prazo é contado do momento em que a mensagem entra no moderation-gate até o momento em que a decisão está tomada e registrada. O prazo de 2 seconds é um limite superior, não uma meta média. O objetivo prático é que a grande maioria das mensagens limpas seja decidida bem antes disso, e que a cauda lenta também respeite o limite.

Há uma consequência que a equipe já discutiu e deixou assentada: o prazo de 2 seconds se aplica só ao caminho das mensagens não sinalizadas. Mensagens sinalizadas não têm prazo de 2 seconds, porque dependem de uma pessoa. Elas têm outras metas de operação, descritas mais abaixo, mas essas metas nunca autorizam a liberação automática.

### O que fica fora do contrato

O moderation-gate não decide política de conteúdo sozinho. Quais regras sinalizam uma mensagem é configuração do evento, mantida por produtores e gerentes de comunidade. O moderation-gate aplica essa configuração e respeita o resultado. Ele também não é responsável por entregar a mensagem aprovada aos clientes; isso é papel da camada de difusão em tempo real sobre WebSockets. O moderation-gate só emite a decisão e o evento de que a mensagem foi liberada.

Também fica fora o histórico de longo prazo de moderação para relatórios. O gate grava o que precisa para auditoria e para reconstruir a decisão, mas relatórios agregados são outra parte do sistema.

## Fluxo das mensagens

O caminho de uma mensagem, em linhas gerais, é este.

O participante envia a mensagem pelo cliente em Next.js, que a manda por uma conexão WebSocket ao servidor Phoenix. O canal do evento valida o formato básico, identifica o participante e o evento, e entrega a mensagem ao moderation-gate. Nesse ponto a mensagem ainda não é visível para ninguém além de quem a enviou, e mesmo para quem enviou ela aparece como pendente, sem prometer publicação.

Dentro do moderation-gate, a mensagem passa pela avaliação de sinalização. A avaliação combina as regras configuradas para o evento: listas de termos bloqueados, sinais de abuso repetido do mesmo participante, links, indícios de spam e denúncias de outros participantes quando existirem. O resultado é binário para fins de roteamento: sinalizada ou não sinalizada. Qualquer dúvida na avaliação conta como sinalizada. Esse é o princípio de projeto mais importante do gate, e vem da obrigação de retenção: na incerteza, retém.

Se a mensagem não for sinalizada, o gate segue o caminho de aprovação automática. Ele registra a decisão, marca a mensagem como aprovada e emite o evento de liberação. Tudo isso precisa caber nos 2 seconds. Se a mensagem for sinalizada, o gate a coloca na fila de revisão humana, registra o motivo da sinalização e avisa os moderadores conectados. A mensagem fica retida até que uma pessoa a aprove, a rejeite ou a edite conforme as ações permitidas.

### Processos e supervisão no lado Elixir

Cada evento ao vivo tem seu próprio conjunto de processos de moderação, supervisionados de forma que a falha de um evento não derrube o gate dos outros. A intenção é isolar eventos grandes de eventos pequenos: um pico de mensagens em um evento enorme não deve atrasar a decisão automática em outro evento. Dentro de um evento, a avaliação das mensagens pode rodar em paralelo, mas a ordem de publicação deve respeitar a ordem de chegada quando isso importar para a leitura de perguntas e respostas. Quando a ordem estrita e a latência conflitam, a latência das mensagens limpas vence, e a ordem é reconstruída pelo carimbo de chegada que o gate grava.

O estado durável vive no CockroachDB. O gate grava a mensagem, o resultado da avaliação e a decisão antes de emitir qualquer evento de liberação. A ordem importa: primeiro grava, depois anuncia. Assim, se o processo cair entre os dois passos, a recuperação consegue ver que a decisão existe e reemitir o evento, em vez de perder a mensagem ou, pior, publicar sem registro.

### Idempotência

A mesma mensagem pode chegar mais de uma vez ao gate, por reenvio do cliente após queda de conexão ou por reentrega interna. O gate trata a entrada como idempotente por identificador da mensagem: uma segunda chegada da mesma mensagem não cria uma segunda decisão, não duplica itens na fila de revisão e não reinicia o relógio do prazo de 2 seconds. Se a primeira chegada já virou retida, a segunda também resulta em retida, nunca em aprovada.

## Revisão humana das mensagens sinalizadas

A fila de revisão é o outro lado do gate. Toda mensagem sinalizada entra nela, e só sai por ação humana.

Os moderadores veem a fila no painel do produtor e do gerente de comunidade, construído em Next.js e alimentado em tempo real pelos mesmos canais WebSocket. Cada item mostra o texto, o autor no nível de detalhe que a política do evento permitir, o motivo da sinalização e o contexto mínimo para decidir, como a pergunta à qual a mensagem responde, se for o caso. As ações são aprovar, rejeitar e, quando o evento permitir, editar antes de aprovar. Toda ação grava quem fez, quando fez e o que decidiu.

Quando dois moderadores abrem o mesmo item, o gate evita decisões conflitantes: a primeira decisão gravada vale, e a segunda pessoa recebe a informação de que o item já foi tratado. Essa checagem acontece no banco, não só na interface, porque a interface pode estar desatualizada.

### Retenção sem moderador disponível

Pode acontecer de a fila crescer e ninguém estar olhando. O comportamento exigido é simples e desconfortável: as mensagens continuam retidas. Não há expiração que as libere, não há modo de transbordo que as aprove sozinhas e não há exceção para eventos prestes a acabar. O que o gate pode fazer é tornar o problema visível: alertar o produtor, destacar a fila no painel, e permitir que o produtor convoque mais moderadores ou ajuste as regras de sinalização do evento para reduzir o volume dali em diante.

Ajustar as regras muda o que é sinalizado daí para frente. Não reavalia para liberar o que já está retido. Se alguém quiser liberar em massa o que está na fila, isso é uma ação humana explícita, registrada, feita por quem tem permissão, e não um efeito colateral de mudança de configuração.

### Prioridade e ordenação da fila

A fila não precisa ser estritamente por ordem de chegada. Perguntas que o palestrante está prestes a ler, denúncias de vários participantes e mensagens de participantes com histórico limpo podem subir. Mas a priorização só reordena a fila; ela nunca tira item da fila. A ordem é um recurso de conveniência para os moderadores. A garantia de retenção independe dela.

### Moderadores e permissões

Nem todo usuário do painel pode revisar. Há papéis: produtor do evento, gerente de comunidade e moderador. As permissões exatas ficam na configuração do evento. Para esta especificação basta saber que a ação de liberar uma mensagem sinalizada exige papel com permissão de moderação, e que o gate confere isso no servidor a cada ação, sem confiar no que o cliente afirma.

## Decisão automática e prazo

Esta seção trata do caminho rápido e do prazo de 2 seconds.

O caminho rápido existe para que a conversa do evento pareça viva. Participantes que mandam uma pergunta ou comentário comum esperam vê-la rapidamente. Se a aprovação automática demorar demais, o evento parece travado e os produtores começam a desligar a moderação para ganhar velocidade, que é exatamente o que o produto quer evitar. Por isso o prazo de 2 seconds é uma exigência de produto, não só uma preferência técnica.

### O que conta dentro dos 2 seconds

O relógio cobre a avaliação de sinalização, a gravação da decisão no banco e a emissão do evento de que a mensagem foi aprovada. Não cobre o tempo de rede entre o cliente e o servidor, nem o tempo de difusão até cada participante, que dependem de fatores fora do gate. Quem mede o gate deve medir dentro do servidor, entre a entrada da mensagem e a decisão registrada, e deve medir a distribuição, não só a média. Uma média boa com uma cauda ruim descumpre o contrato.

### Orçamento de tempo

Para caber no prazo, cada etapa do caminho rápido precisa ter um orçamento próprio e nenhuma etapa pode consumir o prazo inteiro. A avaliação das regras deve ser local e barata sempre que possível: listas e padrões carregados em memória, atualizados quando a configuração do evento muda, em vez de consultados no banco a cada mensagem. Consultas ao CockroachDB no caminho rápido devem se limitar ao necessário para gravar a decisão e para ler o mínimo de contexto, como o histórico recente do autor. Chamadas a serviços externos de classificação, se o evento usar algum, entram com tempo limite próprio, bem menor que o prazo total.

### Quando a avaliação não termina a tempo

Esta é a regra que mais gera confusão, então fica explícita. Se a avaliação de uma mensagem não terminar dentro do orçamento, a mensagem não é aprovada automaticamente por falta de resposta. Ela é tratada como sinalizada e vai para a revisão humana, com o motivo registrado como avaliação incompleta. Aprovar por timeout violaria a obrigação de retenção, porque não sabemos se a mensagem seria sinalizada. A consequência é que o prazo de 2 seconds é protegido por um desvio seguro: no pior caso, uma mensagem limpa vai para a fila humana e demora mais, mas nenhuma mensagem duvidosa passa.

Isso significa também que degradação de desempenho se manifesta como crescimento da fila de revisão, e não como mensagens perigosas publicadas. Quem opera deve olhar a proporção de mensagens retidas por avaliação incompleta como sinal de saúde do gate.

### Aprovação automática não é aprovação cega

Uma mensagem não sinalizada aprovada pelo gate continua sujeita a ação posterior. Moderadores podem remover uma mensagem já publicada, e denúncias de participantes depois da publicação podem reabrir a análise. O gate trata a remoção posterior como um evento normal, que grava a decisão nova e emite o evento de retirada. A aprovação automática diz apenas que, no momento da entrada, nada indicou problema.

## Falhas, degradação e recuperação

O gate precisa se comportar bem quando partes do sistema falham, porque eventos grandes têm picos e falhas acontecem justamente nos picos.

Se o CockroachDB ficar lento ou indisponível para escrita, o gate não pode aprovar sem registrar. A regra de gravar antes de anunciar continua valendo. Nesse caso as mensagens novas ficam pendentes, sem serem publicadas, e o painel do produtor mostra o estado degradado. É melhor um evento parecer lento do que publicar mensagens sem trilha de auditoria. Quando o banco voltar, as mensagens pendentes são processadas pelo caminho normal; as que já tiverem estourado o prazo de 2 seconds são tratadas como sinalizadas por avaliação incompleta, pelo mesmo princípio da seção anterior.

Se um processo de moderação de um evento cair, o supervisor o reinicia e o estado é reconstruído a partir do banco. O que estava retido continua retido. O que estava aprovado e registrado não é reaprovado nem reanunciado em duplicidade, graças à idempotência por identificador da mensagem. O que estava em avaliação sem decisão gravada é reavaliado, e se o prazo já passou, vai para revisão humana.

Se a conexão WebSocket de um moderador cair, as ações dele não se perdem silenciosamente: o cliente só mostra uma ação como concluída depois da confirmação do servidor. Ao reconectar, o painel ressincroniza a fila com o estado do servidor, sem confiar no estado local.

### Picos de volume

Em picos, o gate deve preferir proteger o isolamento entre eventos e a retenção das mensagens sinalizadas a proteger a ordem estrita. Se o volume exceder a capacidade de um evento, o gate aplica contrapressão no canal daquele evento, pode limitar a taxa por participante e deve deixar claro ao cliente que a mensagem foi recebida e está pendente. Descartar mensagens em silêncio não é aceitável. Se for preciso recusar, o participante recebe uma resposta explícita de que a mensagem não foi aceita.

### Reinício e implantação

Atualizações do sistema durante um evento ao vivo devem evitar derrubar o gate. Quando for inevitável reiniciar, vale a mesma ideia: o estado durável está no banco, e a recuperação deve ser segura. Nenhuma implantação pode mudar a regra de que sinalizada significa retida.

## Observabilidade, testes e pontos em aberto

O gate deve emitir métricas suficientes para saber, durante o evento, se está cumprindo as duas obrigações. Para o prazo, a distribuição do tempo entre entrada e decisão das mensagens não sinalizadas, com atenção à cauda e à fração que passa de 2 seconds. Para a retenção, o tamanho e a idade da fila de revisão, a proporção de mensagens retidas por cada motivo, e um contador de qualquer mensagem sinalizada que tenha sido publicada sem decisão humana, que deve ser sempre zero. Se esse contador sair de zero, é incidente, não alerta de rotina.

Os logs devem permitir reconstruir por que uma mensagem específica foi retida ou aprovada: qual regra a sinalizou, qual era a configuração do evento naquele momento, quem decidiu e quando. Isso serve tanto para suporte quanto para disputas com produtores que perguntam por que uma pergunta não apareceu.

### Testes que importam

Os testes mais valiosos são os de propriedade sobre as duas obrigações. Primeiro: para qualquer sequência de mensagens, falhas injetadas e reinícios, nenhuma mensagem sinalizada chega ao estado publicado sem uma ação humana registrada. Segundo: sob carga realista, as mensagens não sinalizadas recebem decisão automática dentro de 2 seconds, e as que não conseguem acabam retidas por avaliação incompleta, nunca aprovadas por timeout. Terceiro: chegadas duplicadas da mesma mensagem não geram decisões duplicadas nem liberam algo antes retido.

Vale também testar o comportamento com a fila de revisão cheia e sem moderadores, para confirmar que nada vaza, e testar a concorrência entre dois moderadores sobre o mesmo item, para confirmar que só uma decisão vale.

### Pontos em aberto

Algumas coisas ainda não foram decididas e não devem ser tratadas como fechadas por quem ler esta nota.

- Qual a meta de tempo para a revisão humana de mensagens sinalizadas. Hoje só se sabe que não há liberação automática; a meta de atendimento ainda precisa ser combinada com operação de eventos.
- Se o autor deve ser avisado quando a mensagem foi retida, e com que nível de detalhe, para não ensinar a contornar as regras.
- Como tratar pedidos de reconsideração de mensagens rejeitadas.
- Quanto contexto do autor o moderador deve ver por padrão, respeitando privacidade e as políticas de cada evento.
- Se classificadores externos entram no caminho rápido ou só em uma segunda passada, dado o prazo de 2 seconds.

Enquanto isso não estiver resolvido, quem implementar deve seguir a regra conservadora: na dúvida, retém, e o prazo de 2 seconds vale só para o que não for sinalizado.
