---
id: 01JTC07D85JTFHHJX0QT13RQQR
created: 2025-05-03T18:18-03:00
---

# qa-queue: referência da listagem de perguntas pendentes

Nota de referência sobre a `qa-queue`, a fila de perguntas do TownHall Pulse. Escrevi com pressa, para não precisar redescobrir isto toda vez. O fato central: moderadores listam as perguntas que estão esperando na `qa-queue` chamando `GET /api/v1/events/:event_id/questions?status=pending`. Todo o resto aqui gira em volta dessa chamada: quem usa, o que esperar dela, como ela se relaciona com o tempo real e onde costuma dar problema. Onde eu não sei o detalhe exato (limites, nomes de campos, tamanhos de página), digo que não sei em vez de chutar.

O projeto roda enquetes ao vivo e perguntas e respostas para eventos virtuais grandes, com moderação em tempo real. Quem usa são produtores de evento e gerentes de comunidade. O backend é Elixir com Phoenix, a comunicação ao vivo usa WebSockets, os dados ficam no CockroachDB e a interface web é em Next.js. A `qa-queue` fica no meio disso: recebe as perguntas enviadas pelo público e entrega para os moderadores decidirem o que fazer com cada uma.

A regra prática para quem chega agora: se a tela do moderador mostra uma pergunta esperando aprovação, ela veio da `qa-queue`, e a forma de pedir a lista inteira de uma vez é a chamada acima. O canal de WebSocket serve para atualizações incrementais, não para carregar o estado inicial.

## O que é a qa-queue

A `qa-queue` é o componente que guarda as perguntas do público enquanto elas ainda não foram tratadas. Pense nela como uma caixa de entrada por evento. Cada pergunta entra num estado de espera e sai dele quando um moderador age. O que o moderador pode fazer depende da configuração do evento, mas em geral é aprovar, rejeitar ou deixar para depois. Não vou listar aqui os nomes exatos de cada estado além de `pending`, porque o único valor que esta nota garante é esse.

Duas coisas importam para entender a `qa-queue`.

Primeiro, ela é sempre escopada por evento. Não existe uma fila global que misture perguntas de eventos diferentes. O identificador do evento vai no caminho da chamada, na posição `:event_id`, e é isso que define de qual fila você está falando. Se você pedir a lista com o evento errado, vai ver a fila errada, e isso é mais comum do que parece quando o produtor está com dois eventos abertos em abas diferentes.

Segundo, a `qa-queue` não decide sozinha o que é bom ou ruim. Ela guarda e entrega. A moderação em si, ou seja, a decisão humana (ou as regras automáticas que o evento tenha ligado), acontece em cima da fila. Quando alguém diz que a fila está "cheia", quer dizer que há muitas perguntas ainda em espera, não que o componente esteja com problema.

A ideia de fila aqui é mais conceitual do que técnica. Não é uma fila de mensagens no sentido de um broker. As perguntas são registros persistidos no banco, e a "fila" é a visão filtrada desses registros pelo estado de espera. Isso explica por que a listagem é uma consulta comum por HTTP e por que dá para filtrar por status.

## Listagem de perguntas pendentes

A chamada que os moderadores usam para ver o que está esperando é esta:

```http
GET /api/v1/events/:event_id/questions?status=pending
```

Substitua `:event_id` pelo identificador do evento. O parâmetro de consulta `status=pending` pede só as perguntas que ainda aguardam moderação. Sem esse filtro, a mesma rota pode devolver perguntas em outros estados, então não omita o parâmetro quando a intenção é mostrar a fila de espera.

O que a resposta representa: a lista de perguntas do evento indicado cujo estado é pendente. Quem consome (a interface do moderador, em Next.js) usa essa lista para montar a tela de fila. Se a lista vem vazia, significa que não há nada esperando naquele momento naquele evento, não que a rota falhou.

O que eu não registro aqui, de propósito: o formato exato de cada item, a ordem garantida, o tamanho máximo de página e o esquema de paginação. Se você precisar de algum desses, leia o controlador no código do Phoenix em vez de confiar em memória minha. Marquei isso como lacuna na seção final.

### Quando usar essa chamada

Use para carregar o estado inicial da fila quando o moderador abre a tela, quando ele recarrega a página e quando o cliente reconecta depois de uma queda e precisa se alinhar com o servidor. Não use em laço rápido para simular tempo real. Para isso existe o canal de WebSocket, e martelar a rota HTTP em evento grande só aumenta carga no banco sem ganhar nada.

### Quando não usar

Não use essa listagem para mostrar perguntas ao público. A visão do público é outra coisa, normalmente só com o que já foi aprovado. A listagem de pendentes é uma visão de moderação e pode conter conteúdo que ninguém deveria ver ainda. Se algum dia essa rota aparecer exposta numa tela de participante, trate como bug de segurança, não como detalhe de interface.

## Como o moderador usa isso no dia a dia

O fluxo típico, do jeito que eu entendi conversando com quem opera os eventos:

O moderador entra na tela do evento. A interface chama a listagem de pendentes e desenha a fila. A partir daí, novas perguntas aparecem pelo canal ao vivo, e as que o moderador trata saem da lista. Ao final de um bloco de perguntas e respostas, o produtor costuma olhar o que sobrou na fila para decidir se vale estender o bloco ou encerrar.

Em eventos grandes, vários moderadores olham a mesma fila ao mesmo tempo. Isso gera duas situações que valem anotar.

A primeira é a disputa: dois moderadores abrem a mesma pergunta e agem quase juntos. O comportamento esperado é que a última ação não corrompa o estado, mas a interface pode mostrar por um instante uma pergunta que outro já tratou. Não prometo aqui nenhum mecanismo de trava específico; só registro que o problema existe e que a atualização ao vivo é o que o reduz.

A segunda é a divergência entre telas: um moderador vê uma fila um pouco diferente da do colega porque a mensagem ao vivo de um deles chegou atrasada ou se perdeu. A saída simples é recarregar a listagem de pendentes. Se recarregar resolve, o problema era de entrega no canal, não de dados.

### Gerentes de comunidade

Gerentes de comunidade também olham a fila, mas com objetivo diferente do produtor. O produtor quer ritmo: o que entra no ar agora. O gerente de comunidade quer saber o que o público está perguntando, inclusive o que não vai ao ar. Para os dois, a chamada de pendentes é o ponto de partida, mas as expectativas sobre o que significa "vazia" mudam. Para o produtor, fila vazia é bom. Para o gerente, pode ser sinal de que o público não está conseguindo enviar.

## Tempo real e WebSockets

A listagem por HTTP dá um retrato num instante. O que mantém o retrato atualizado é o canal de WebSocket do Phoenix. Em termos gerais:

- A listagem carrega o estado inicial.
- O canal avisa de mudanças depois disso: pergunta nova entrando, pergunta tratada saindo.
- Na reconexão, o cliente deve voltar a chamar a listagem, porque mensagens enviadas durante a queda não são reenviadas automaticamente.

Esse último ponto é a causa de muita confusão. Quem assume que o canal entrega tudo acaba com uma fila que mostra pergunta velha ou não mostra pergunta nova. A regra é: o canal é otimização, a listagem é a verdade. Se houver dúvida, a listagem ganha.

Por ser Elixir, o canal aguenta muitas conexões simultâneas sem drama, e isso é uma das razões da escolha da pilha. Mas isso não significa que a consulta ao banco por trás da listagem seja de graça. Em um evento com muita gente e muitos moderadores recarregando, a listagem vira a parte cara. Por isso insisto em não usar a rota HTTP como substituto do canal.

### Ordem das mensagens

Não assuma que mensagens ao vivo chegam na mesma ordem em que as ações aconteceram, principalmente com vários moderadores. Se a interface precisa de ordem estável, ela deve ordenar localmente com base nos dados da própria pergunta, não na ordem de chegada. Não sei dizer qual campo é usado hoje; confira no código do cliente.

### Interface em Next.js

A parte em Next.js é só consumidora. Ela chama a listagem, abre o canal e mantém o estado local. Um erro comum de quem mexe nessa tela é guardar a fila em dois lugares, um vindo da listagem e outro vindo do canal, e esquecer de reconciliar. Prefira uma única fonte de estado e deixe a listagem substituí-la por inteiro quando for recarregada, em vez de tentar mesclar.

## Armazenamento e consistência no CockroachDB

As perguntas ficam no CockroachDB. Isso traz algumas consequências que vale ter em mente quando a fila se comportar de forma estranha.

O CockroachDB é distribuído e usa transações com isolamento forte. Na prática, isso significa que a leitura da listagem enxerga um estado consistente, mas transações que competem pelos mesmos registros podem ser reexecutadas. Duas ações de moderação na mesma pergunta, em sequência muito próxima, são o caso clássico. O código do lado Elixir precisa estar preparado para repetir a operação quando o banco pedir. Se você vir lentidão pontual em picos de moderação, comece por aí antes de culpar a rede.

Outro ponto: a consulta de pendentes filtra por evento e por estado. Se algum dia a listagem ficar lenta em eventos grandes, o suspeito natural é a falta de um índice que sirva a esse filtro, ou um plano de consulta que não o esteja usando. Não afirmo que o índice existe ou não; verifique o esquema antes de mexer. Mudar índice em produção durante um evento ao vivo é má ideia. Faça fora do horário.

### Leitura logo depois da escrita

Uma pergunta enviada pelo público deve aparecer na listagem de pendentes quase imediatamente, mas "quase" é a palavra. Se um teste automatizado envia uma pergunta e já em seguida lista, pode haver janela em que ela ainda não aparece, dependendo de como a escrita e a leitura são feitas. Em teste, espere ou tente de novo com limite; não escreva um teste que assume visibilidade instantânea sem checar como a escrita é confirmada.

### Dados sensíveis

Perguntas do público podem conter dados pessoais ou conteúdo ofensivo. A listagem de pendentes é justamente o lugar onde esse conteúdo aparece antes de qualquer filtro humano. Cuidado com logs: não registre o texto completo das perguntas em log de aplicação, e não cole respostas dessa rota em chats ou tickets sem necessidade.

## Armadilhas e dúvidas comuns

Reúno aqui o que mais confunde, em formato de pergunta e resposta curta.

**A fila está vazia mas o público diz que enviou pergunta. O que olhar?**
Primeiro confirme o `:event_id` da chamada: é a causa mais frequente. Depois confirme que o parâmetro `status=pending` está presente e escrito como está na rota acima. Se estiver tudo certo, a pergunta pode ter sido tratada por outro moderador ou por regra automática, ou pode ter falhado na entrada. Nesse último caso, o problema está antes da `qa-queue`, no envio.

**A listagem mostra perguntas que já foram tratadas.**
Sem o filtro de status, a rota devolve mais do que pendentes. Veja se o filtro foi perdido numa montagem de URL. Se o filtro está lá, pode ser cache de cliente ou de intermediário. A listagem de moderação não deve ser cacheada de forma agressiva.

**O canal mostra uma pergunta nova mas a listagem não.**
Trate como atraso de leitura ou de consistência e tente de novo. Se persistir, é bug real e vale abrir investigação, guardando o evento, o horário aproximado e o que cada lado mostrava.

**Posso filtrar por outros estados com a mesma rota?**
A rota aceita o parâmetro de status, então é razoável supor que outros valores existam, mas esta nota só garante `pending`. Leia o código antes de depender de qualquer outro valor.

**Dá para usar a listagem como fonte para relatório pós-evento?**
Para pendentes, não: ela mostra o que falta tratar, não o histórico. Relatório precisa de uma visão que inclua todos os estados.

### Erros de quem está começando

Copiar a URL com `:event_id` literal, sem trocar pelo identificador, e estranhar o erro. Esquecer de reautenticar depois de uma sessão longa de evento e achar que a fila sumiu. Testar contra o evento errado em ambiente de homologação porque as abas parecem iguais. Nenhum desses é problema da `qa-queue`, mas todos aparecem como "a fila está estranha".

## Como depurar uma fila que parece errada

Ordem que costumo seguir, da causa mais barata para a mais cara:

1. Confirmar o evento. Olhar o identificador na URL da tela e na chamada.
2. Refazer a listagem de pendentes na mão e comparar com o que a tela mostra. Se a chamada manual está certa e a tela errada, o defeito é no cliente em Next.js ou no canal.
3. Verificar se o canal está conectado. Reconexões silenciosas explicam muito caso de tela atrasada.
4. Olhar logs do lado Phoenix para o evento em questão, procurando reexecução de transação ou erro de banco.
5. Só então suspeitar do CockroachDB, e nesse caso começar pelo plano da consulta de pendentes.

Na maior parte das vezes a resposta está no passo um ou dois. Pular direto para o banco costuma gastar horas.

### Durante um evento ao vivo

Se o problema acontece com o evento no ar, a prioridade não é achar a causa, é devolver aos moderadores uma fila confiável. Recarregar a tela, que refaz a listagem, resolve a maioria dos casos de divergência e custa pouco. Investigação de causa fica para depois. Evite deploy durante evento grande, a menos que o defeito esteja impedindo a moderação.

### Depois do evento

Anote o que aconteceu enquanto a memória está fresca: qual evento, o que os moderadores viram, o que o servidor mostrava. Nota curta basta. Sem isso, o mesmo defeito volta em outro evento e ninguém sabe que já tinha aparecido.

## O que esta nota não cobre

Lacunas conhecidas, para ninguém tomar esta nota por completa:

- Formato exato do corpo da resposta da listagem e dos campos de cada pergunta.
- Regras de paginação e limites de tamanho.
- Ordem garantida dos itens.
- Lista completa dos estados de uma pergunta além de `pending`.
- Rotas de ação do moderador (aprovar, rejeitar e semelhantes) e seus formatos.
- Nomes dos tópicos e eventos do canal de WebSocket.
- Regras de autorização: quem exatamente pode chamar a listagem de pendentes. Pela natureza da rota, deve ser restrito a moderação, mas confirme no código antes de afirmar.
- Esquema das tabelas e índices no CockroachDB.

Quando alguém preencher uma dessas lacunas, atualize esta nota em vez de criar outra sobre o mesmo assunto. Se descobrir que algo acima está errado, corrija aqui mesmo e remova o que não vale mais.

## Resumo para consulta rápida

A `qa-queue` guarda as perguntas do público, por evento, até um moderador tratá-las. Para listar as que aguardam moderação, a chamada é `GET /api/v1/events/:event_id/questions?status=pending`. Use-a para o estado inicial e para realinhar depois de reconexão. Deixe as atualizações incrementais com o canal de WebSocket. Em caso de divergência, a listagem é a verdade e o canal é só atalho. Fila vazia quase sempre significa evento errado ou filtro perdido antes de significar defeito no componente. E a listagem de pendentes é visão de moderação: nunca deve chegar ao público.
