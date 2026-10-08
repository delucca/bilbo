---
id: 01KYEA9SMER8CPJ83C8AT204TF
created: 2026-07-26T01:18-03:00
sources:
  - "code: app/views/calendar/show.html.erb"
---

# Design do frontdesk-ui: grade do calendário com Turbo Frames

Anotação de design do frontdesk-ui, a camada de interface usada pela recepção das clínicas pequenas que usam o ClinicSlotter. O ponto central: o frontdesk-ui renderiza a grade do calendário com Turbo Frames na view `app/views/calendar/show.html.erb`. Tudo o que está abaixo parte dessa escolha. Quem for mexer na grade deve começar por esse arquivo e só depois olhar controllers, helpers e jobs.

O objetivo da tela é simples: a pessoa da recepção precisa ver, de relance, quem atende quando, qual sala está livre e onde ainda cabe um horário. Ela não quer esperar a página inteira recarregar toda vez que muda uma consulta. Também não queremos montar um front-end separado em JavaScript para isso, porque o time é pequeno e o projeto é Rails de ponta a ponta. Turbo Frames deram o meio-termo: o HTML continua sendo produzido no servidor, mas só o pedaço que mudou é trocado.

## Visão geral da tela

O frontdesk-ui é uma aplicação server-rendered dentro do mesmo app Rails do restante do ClinicSlotter. Não existe um serviço de front-end à parte. As telas são ERB, o estado vive no MySQL e o que precisa acontecer fora do ciclo da requisição vai para o Sidekiq. A hospedagem é no Heroku, então qualquer coisa que dependa de disco local ou de processo de longa duração na web não serve.

A tela do calendário mostra uma grade. As colunas representam clínicos ou salas, dependendo da visão escolhida pela pessoa da recepção, e as linhas representam faixas de horário. Cada célula pode estar em um destes estados: livre, ocupada por uma consulta, bloqueada por indisponibilidade do clínico, bloqueada por restrição de sala, ou em conflito. O estado "em conflito" não deveria aparecer em condições normais, mas existe porque dados importados de fora podem chegar inconsistentes, e preferimos mostrar o problema a esconder.

A view `app/views/calendar/show.html.erb` é a dona da estrutura geral: cabeçalho com a navegação de dias, a barra de filtros e o container da grade. A grade em si fica dentro de um frame. Os filtros ficam fora do frame de propósito, para que a pessoa não perca o que selecionou quando só a grade for trocada.

### O que é responsabilidade da view e o que não é

A view monta a estrutura e delega a decisão de "o que pode ser feito nesta célula" para objetos de apresentação. Ela não deve conter regra de agendamento. A regra de disponibilidade do clínico e de uso de sala fica no domínio, fora da camada de interface. Se alguém sentir vontade de calcular conflito dentro do ERB, é sinal de que a lógica está no lugar errado.

Isso importa porque a mesma regra precisa valer para a interface, para os jobs em segundo plano e para a integração HL7 FHIR. Se a interface tivesse uma regra própria, ela divergiria das outras e a recepção veria horários "livres" que o servidor recusa na hora de gravar.

## Como a grade usa Turbo Frames

A grade é envolvida por um Turbo Frame com identificador estável. O identificador não muda entre requisições, porque o Turbo precisa casar o frame da resposta com o frame que já está na página. Se o identificador variar, por exemplo por incluir algo que muda a cada renderização, a troca falha em silêncio e a pessoa fica olhando uma grade velha. Esse foi o primeiro tipo de bug que apareceu e vale a pena lembrar dele.

Dentro do frame ficam as linhas de horário. Cada célula que representa uma consulta ou um horário livre é clicável. O clique segue um link que aponta para um segundo frame, o do painel lateral de detalhe e edição da consulta. Assim a grade e o painel se atualizam de forma independente: salvar uma consulta troca o painel e, em seguida, a grade é recarregada para refletir o novo estado.

### Navegação dentro do frame

Os links de navegação de dia, anterior e próximo, apontam para o frame da grade. Ao clicar, só a grade é trocada, o cabeçalho e os filtros permanecem. Para que o botão voltar do navegador funcione bem, a navegação de dia precisa promover a visita para a barra de endereço. Sem isso, a URL fica parada no dia anterior enquanto a grade mostra outro dia, e quem copia o link da página manda o colega para o dia errado. Esse comportamento está configurado nos links de navegação e deve ser mantido quando a view for refatorada.

Os filtros, por outro lado, são um formulário comum que dispara uma visita completa do Turbo com a grade como alvo. Mantivemos o formulário simples. A tentação de atualizar a grade a cada tecla digitada nos filtros foi descartada, porque gera muitas requisições e a recepção quase nunca precisa disso.

### Carregamento preguiçoso de partes pesadas

Algumas partes da tela custam caro para calcular, em especial o resumo de ocupação por sala. Essas partes são frames com carregamento preguiçoso: a página sai rápida com um espaço reservado e o conteúdo chega em seguida. O espaço reservado tem altura parecida com a do conteúdo final, para a grade não pular quando o frame carrega. Esse detalhe de layout parece pequeno, mas a recepção reclamou muito do pulo antes de ele ser corrigido.

## Fluxo de dados e atualização

A requisição que renderiza a grade carrega do banco as consultas do período visível, as janelas de disponibilidade dos clínicos e as reservas de sala. O controller reúne isso e entrega à view já organizado por célula. A view apenas percorre a estrutura. Evitamos consultas dentro do ERB para não cair em N+1, que no MySQL com muitos clínicos aparece rápido como lentidão visível.

Quando a recepção cria ou move uma consulta, a ação do controller valida, grava e responde com os fragmentos que mudaram. Em vez de redirecionar para a página inteira, a resposta atualiza o painel lateral e pede o recarregamento do frame da grade. O resultado percebido é uma tela que muda "no lugar", sem piscar.

### O que vai para o Sidekiq

Nada que dependa de sistema externo acontece dentro da requisição da interface. A sincronização com sistemas de terceiros via HL7 FHIR, notificações e recálculos pesados são enfileirados no Sidekiq. A interface mostra o estado local imediatamente e indica, de forma discreta, quando algo ainda está pendente de sincronização. A recepção não deve ficar esperando uma resposta externa para confirmar um horário com o paciente ao telefone.

Uma consequência: a grade pode, por um curto intervalo, mostrar uma consulta que ainda não foi confirmada do outro lado. Aceitamos isso. O que não aceitamos é a grade mostrar como livre um horário que já foi tomado localmente; por isso a gravação local é a fonte de verdade para a tela.

### Concorrência entre duas pessoas na recepção

Em clínicas com mais de uma pessoa na recepção, duas podem tentar ocupar o mesmo horário quase ao mesmo tempo. A defesa não está na interface, e sim na validação no servidor, com verificação no momento de gravar. Quando a segunda tentativa falha, a resposta devolve o painel com uma mensagem clara de que o horário acabou de ser ocupado e recarrega a grade para mostrar o estado atual. A interface nunca deve assumir que o que estava na tela ainda vale.

## Armadilhas conhecidas

Esta seção junta o que já deu problema ou quase deu. É o que eu gostaria de ter lido antes de mexer na grade.

### Identificadores de frame

Já foi dito, mas repito por ser a causa mais comum de "a grade não atualiza": o identificador do frame da resposta tem que ser igual ao do frame na página. Qualquer resposta de erro, por exemplo um redirecionamento para uma página de login, uma página de erro do servidor ou uma tela sem o frame, faz o Turbo mostrar o aviso de conteúdo ausente dentro do frame. Ao testar, olhar o console do navegador antes de suspeitar do back-end.

### Sessão expirada

Se a sessão da pessoa expira com a tela aberta, a próxima navegação dentro do frame recebe a página de login dentro do frame. Isso fica feio e confuso. O tratamento adequado é detectar a resposta de autenticação e fazer uma visita completa em vez de trocar só o frame. Isso precisa continuar funcionando quando mexermos na autenticação.

### Cache de fragmentos

Há cache de fragmentos em partes da grade. A chave do cache precisa incluir tudo o que altera o visual da célula: a consulta, a disponibilidade do clínico e a restrição de sala. Já houve caso de célula mostrando horário livre depois de uma alteração de disponibilidade, porque a chave só dependia da consulta. Ao adicionar um novo tipo de bloqueio, revisar as chaves de cache faz parte da tarefa, não é opcional.

### Fuso horário

Clínicas pequenas costumam atender em um único fuso, mas o servidor guarda tudo de forma normalizada. A conversão para exibição tem que acontecer na camada de apresentação, usando o fuso da clínica e não o do navegador nem o do servidor no Heroku. Mistura de fusos produz consultas deslocadas na grade, e o erro só aparece perto de mudanças de horário de verão, quando ninguém está olhando.

### Acessibilidade e teclado

A recepção trabalha muito com teclado. As células da grade precisam ser alcançáveis por tabulação e ter rótulos que descrevam clínico, sala e horário, não só a cor. Cor sozinha não pode ser o único sinal de estado, porque parte dos usuários tem dificuldade para distinguir os tons e porque telas de clínica costumam ser antigas e com contraste ruim.

## Decisões e alternativas descartadas

### Por que Turbo Frames e não um front-end separado

Um front-end de página única daria mais controle sobre interações finas, como arrastar uma consulta de um horário para outro. Em troca, exigiria uma API própria, duplicaria validação, aumentaria o custo de manter o deploy no Heroku e pediria uma competência que o time não tem sobrando. Turbo Frames resolvem a maior parte do que a recepção precisa com muito menos peças. Se a necessidade de arrastar e soltar se tornar central, a conversa deve ser reaberta, mas o ponto de partida é medir o quanto a recepção realmente usa a função antes de mudar de arquitetura.

### Por que a grade é um frame e não a página inteira

Trocar a página inteira funcionaria, mas perderíamos a posição de rolagem, o foco e os filtros preenchidos a cada ação. Isolar a grade em um frame preserva tudo isso e reduz o volume de HTML trafegado. O custo é ter que cuidar dos identificadores e da promoção de URL, como descrito acima. Achamos o custo aceitável.

### Por que a regra de agendamento fica fora da view

Já explicado, mas é a decisão que mais protege o sistema: a view em `app/views/calendar/show.html.erb` só apresenta. Disponibilidade e restrição de sala são decididas no domínio, e a interface pergunta a ele. Isso mantém a tela, os jobs do Sidekiq e a integração FHIR concordando entre si.

### Atualização em tempo real

Consideramos empurrar atualizações da grade para todas as pessoas conectadas, de modo que uma alteração feita em um computador aparecesse nos outros. Por enquanto não fazemos isso. Em clínicas pequenas o número de pessoas simultâneas é baixo, a validação no servidor já cobre o conflito, e o recarregamento da grade ao salvar é suficiente. Adicionar tempo real traria uma conexão persistente a mais para operar no Heroku e mais casos de borda para testar. Fica como possibilidade futura, não como pendência.

## Como testar mudanças na grade

Testes de sistema cobrem os fluxos principais: abrir o calendário, navegar entre dias, criar uma consulta, tentar ocupar um horário já tomado e ver a mensagem de conflito. Ao mudar a estrutura de frames, rodar esses testes e abrir a tela manualmente com dados realistas, incluindo um clínico com disponibilidade fragmentada e uma sala com restrição. Testes que só olham o HTML do servidor não pegam o problema de identificador de frame, porque o erro acontece no navegador.

Para mudanças visuais, conferir também com a janela estreita, já que alguns balcões usam monitores pequenos. A grade deve rolar horizontalmente dentro do próprio container, sem quebrar o cabeçalho nem os filtros.

### Checklist rápido antes de abrir um pedido de revisão

- O identificador do frame da grade continua igual na página e nas respostas.
- A navegação de dia continua atualizando a barra de endereço.
- Nenhuma regra de agendamento foi parar dentro do ERB.
- As chaves de cache incluem todos os fatores que alteram a célula.
- Horários são exibidos no fuso da clínica.
- A resposta de sessão expirada provoca visita completa, não troca de frame.
- As células continuam acessíveis por teclado e têm rótulo textual.

## Pontos em aberto

Ainda não há um desenho final para a visão semanal por clínico quando o número de clínicos cresce muito; hoje a grade fica larga e a rolagem horizontal incomoda. Uma opção é paginar colunas por frame, outra é permitir esconder colunas pelos filtros. Nenhuma foi decidida.

Também falta definir como mostrar de forma mais clara o estado de sincronização com sistemas externos via HL7 FHIR quando ele falha repetidamente. Hoje o indicador é discreto demais para um problema que a recepção precisa notar. A decisão deve envolver quem opera a integração, não só quem cuida da interface.

Por fim, vale revisar periodicamente se o carregamento preguiçoso dos resumos pesados continua compensando. Se o cálculo for otimizado no banco, talvez seja mais simples incluí-lo na renderização principal e remover frames que só existem por causa de lentidão antiga.
