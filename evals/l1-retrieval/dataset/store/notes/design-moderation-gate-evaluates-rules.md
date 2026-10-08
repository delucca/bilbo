---
id: 01KCQ5N1PTK5ZQPE0AQDKDME0J
created: 2025-12-17T19:07-03:00
sources:
  - "code: config/runtime.exs"
---

# Design do moderation-gate (bouncer7)

Nota de design do `moderation-gate`, o componente do TownHall Pulse que decide, em tempo real, o que acontece com cada pergunta e cada resposta de enquete antes de chegar à tela do público. O codinome interno é `bouncer7`; em logs, conversas de plantão e tickets antigos o mesmo componente aparece com esse nome. Aqui uso `moderation-gate`, e quem procurar por `bouncer7` deve cair nesta nota. As regras são avaliadas em um pipeline `Broadway` com `6 processors`, e os ajustes desse pipeline ficam em `config/runtime.exs`. O resto do texto explica por que ficou assim, o que cada parte faz e onde estão as armadilhas. Escrevi com pressa, então algumas seções são curtas e diretas.

## Contexto e problema

O TownHall Pulse roda enquetes e perguntas e respostas para eventos virtuais grandes. Em um evento desses, o volume de mensagens chega em rajadas: o apresentador abre uma enquete, ou pede perguntas, e de repente milhares de pessoas enviam texto ao mesmo tempo. Quem produz o evento e quem cuida da comunidade precisam de moderação que acompanhe esse ritmo, senão a fila humana vira gargalo e o público vê a tela parada, ou pior, vê conteúdo que não devia aparecer.

O `moderation-gate` existe para ficar entre a entrada de conteúdo e a publicação. Ele não substitui o moderador humano. Ele resolve o que é óbvio (spam, ofensas claras, repetição, links proibidos pelo evento) e deixa para a pessoa só o que é ambíguo. O desenho foi pensado para que a decisão automática seja rápida, explicável e reversível.

## Objetivos

Primeiro, latência baixa e previsível entre o envio e a decisão. O público percebe atraso em pergunta ao vivo, e o apresentador depende da fila para escolher o que responder. Segundo, explicabilidade: toda decisão guarda qual regra disparou e por quê, para o moderador conseguir contestar ou reverter. Terceiro, isolamento entre eventos: um evento barulhento não pode atrasar outro. Quarto, configuração por evento, porque cada produtor tem tolerância diferente. Quinto, operação simples: poucas peças móveis, e as que existem têm limites claros.

## Fora de escopo

O `moderation-gate` não faz moderação de imagem ou vídeo; trata texto. Não decide política de banimento de contas de longo prazo, só marca e sinaliza. Não renderiza nada: a interface do moderador e a do público são aplicações em Next.js que consomem o resultado. Também não é um classificador treinado por conta própria; qualquer sinal de modelo externo entra como uma regra a mais, tratada como as demais, e não como caminho especial.

## Visão geral da arquitetura

O fluxo, em linhas gerais: a mensagem chega por WebSocket em um canal do Phoenix, o canal valida o formato e a autorização básica, e entrega o item a um produtor do pipeline. O pipeline `Broadway` distribui os itens entre os processadores, cada um avalia o conjunto de regras do evento correspondente e produz um veredito. O veredito é gravado no CockroachDB e publicado de volta para os canais interessados: o público (se aprovado), a fila do moderador (se for caso de revisão) ou apenas o autor (se for rejeitado, com a mensagem adequada).

Tudo isso roda em Elixir, dentro da mesma aplicação Phoenix que serve os canais. Foi uma escolha consciente: menos saltos de rede, menos serialização, e a supervisão do OTP cuida de reiniciar o que quebrar.

## O pipeline Broadway

O coração do componente é um pipeline `Broadway` com `6 processors`. Os processadores são onde as regras são de fato avaliadas; o resto (produtor, batchers) é infraestrutura em volta. A configuração, incluindo a concorrência dos processadores e os limites de fila, fica em `config/runtime.exs`, e não em código compilado, de modo que dá para ajustar por ambiente sem recompilar a release.

Trecho de referência do que mora lá, só como lembrete de onde olhar:

```elixir
# config/runtime.exs
# moderation-gate (codinome bouncer7)
# pipeline Broadway com 6 processors
```

Os valores reais de fila, timeouts e afins ficam no próprio arquivo e podem mudar; não os copio para esta nota para não ficarem desatualizados. Antes de mexer, leia o arquivo inteiro e entenda quais ajustes são por ambiente.

## Por que Broadway

A alternativa era uma árvore de supervisão própria com processos por evento. Parecia flexível, mas reimplementava o que o `Broadway` já dá: contrapressão, reconhecimento de mensagens, agrupamento em lotes, encerramento ordenado e telemetria pronta. Contrapressão é o ponto decisivo. Em rajada, não queremos aceitar trabalho sem limite e estourar memória; queremos que o pipeline puxe no ritmo em que consegue processar e que a borda (o canal) saiba responder rápido quando está saturada.

Também pesou a legibilidade. Quem entra no projeto entende o pipeline lendo um módulo, com callbacks conhecidos, em vez de reconstruir um desenho caseiro de processos.

## Processadores e concorrência

Os `6 processors` são o número de unidades concorrentes que avaliam regras. O número foi escolhido para equilibrar uso de CPU, pressão sobre o banco e ordem de processamento. Mais processadores aumentam a vazão até certo ponto, mas passam a disputar conexões do banco e a embaralhar a ordem de decisões de um mesmo evento. Menos processadores deixam a latência subir em rajada.

Importante: o número de processadores não é o mesmo que o número de conexões de banco nem o de eventos simultâneos. Se alguém quiser mudar, deve mudar em `config/runtime.exs`, observar a latência de decisão e o tempo de espera por conexão e só então decidir. Não aumente na intuição; o gargalo costuma estar no banco, não na CPU.

## Modelo de regras

Uma regra é uma unidade pequena e pura: recebe o item e o contexto do evento e devolve um resultado entre aprovar, rejeitar, enviar para revisão ou não opinar. As regras são ordenadas, e a avaliação segue essa ordem. Regras puras facilitam teste e fazem o resultado ser o mesmo se o item for reprocessado, o que importa porque o `Broadway` pode reentregar mensagens após falha.

Os tipos de regra mais comuns: lista de termos proibidos, detecção de repetição do mesmo autor, limite de frequência por participante, bloqueio de links, correspondência por padrão e sinais externos. Cada regra declara um nome estável, que aparece no registro da decisão.

## Ordem de avaliação e curto-circuito

A avaliação para na primeira regra que dá um veredito terminal. Rejeição por conteúdo proibido, por exemplo, encerra a avaliação sem gastar tempo com as regras seguintes. A ordem importa e é parte da configuração do evento: regras baratas e muito seletivas vêm antes, regras caras (as que consultam algo externo) vêm por último.

Quando nenhuma regra opina, o comportamento padrão depende do modo do evento: em modo aberto o item é aprovado, em modo restrito ele vai para revisão humana. Esse padrão é decidido por evento, não por regra, para que o produtor entenda sem ambiguidade o que acontece com conteúdo que nenhuma regra reconheceu.

## Configuração por evento

Cada evento tem seu próprio conjunto de regras e seu modo. A configuração é lida do CockroachDB e mantida em cache dentro dos processadores, com invalidação quando o produtor altera algo. O cache evita uma consulta por mensagem, mas cria o risco clássico: uma alteração feita pelo produtor demora um instante para valer. Isso é aceitável, e a interface avisa que a mudança se aplica a partir das próximas mensagens, não retroativamente.

Mudanças de regra durante o evento ao vivo são comuns, porque o produtor reage ao que vê. Por isso a invalidação precisa ser confiável; ver a seção de falhas e armadilhas.

## Persistência no CockroachDB

Cada decisão é gravada com identificador do item, evento, regra responsável, veredito e momento. Usamos CockroachDB pela consistência forte e porque o serviço roda em mais de uma região. O custo é que transações podem ser repetidas por conflito de serialização, então toda escrita do `moderation-gate` precisa ser idempotente e tolerar nova tentativa.

Evitamos transações longas e leituras amplas no caminho quente. A gravação da decisão é uma operação curta e o resto (agregações para painéis, relatórios) lê de outro lugar, para não competir com a moderação ao vivo.

## Entrega em tempo real

Depois da decisão, o resultado é publicado pelos canais do Phoenix sobre WebSockets. O público só recebe o que foi aprovado. O moderador recebe a fila de revisão e também um fluxo de tudo o que foi decidido automaticamente, para poder auditar e reverter. As aplicações em Next.js assinam esses canais e atualizam a tela sem recarregar.

Um cuidado: a publicação acontece depois da gravação, nunca antes. Se a ordem fosse inversa, um item poderia aparecer para o público e, após uma falha, não existir no banco, e o moderador não conseguiria revertê-lo.

## Revisão humana e reversão

Itens ambíguos vão para a fila do moderador, que os aprova ou rejeita. A decisão humana sempre vale mais que a automática e fica registrada junto, sem apagar o histórico: dá para ver que uma regra rejeitou e que uma pessoa reverteu. Isso alimenta o ajuste das regras depois do evento, quando o produtor pergunta por que algo passou ou ficou preso.

Reverter um item já publicado o retira da tela do público por um evento de canal. Reverter uma rejeição o publica como se tivesse sido aprovado naquele momento, sem fingir que foi aprovado antes.

## Contrapressão e saturação

Quando o pipeline satura, a borda precisa reagir sem derrubar o canal. A postura escolhida: o canal continua aceitando conexões, mas responde ao autor que o envio está em processamento e pode demorar, em vez de bloquear. Nada é descartado em silêncio. O que se evita é crescer filas sem limite; os limites estão em `config/runtime.exs`, junto com o resto do `Broadway`.

Se a saturação for persistente, a causa raramente é o número de processadores. Veja primeiro o banco, depois regras caras no começo da ordem, depois o volume anormal de um único evento.

## Falhas e armadilhas

Alguns pontos já conhecidos. Reentrega: o `Broadway` pode entregar o mesmo item mais de uma vez após falha, então qualquer efeito colateral (gravar, publicar) precisa ser idempotente. Cache de regras: se a invalidação falhar, um processador pode avaliar com regras velhas; o sintoma é o produtor dizer que mudou a regra e ainda ver o comportamento anterior. Regras caras: uma regra que chama serviço externo sem limite de tempo pode prender um processador e, com poucos processadores, atrasar todo o evento. Sempre defina limite de tempo e um resultado seguro quando a chamada falha, normalmente não opinar ou enviar para revisão.

Outra armadilha de nomes: `bouncer7` ainda aparece em dashboards e alertas antigos. Quem investiga um alerta com esse nome está olhando para o `moderation-gate`.

## Observabilidade

O pipeline emite telemetria por padrão, e usamos isso para medir o tempo entre o envio e a decisão, o tamanho das filas, a taxa de itens enviados para revisão e a taxa de reversões humanas. Esta última é a melhor medida de qualidade das regras: se moderadores revertem muito uma regra, ela está errada ou larga demais.

Os registros de decisão trazem sempre o nome da regra. Ao investigar uma reclamação, comece pelo item, veja a regra, depois veja a configuração do evento naquele momento.

## Testes

Regras são puras, então testam-se com entradas e saídas diretas, incluindo casos de borda de texto (acentos, maiúsculas, caracteres parecidos, espaços estranhos). O pipeline é testado com um produtor de teste que injeta itens e confere vereditos e efeitos. Testes de carga rodam fora do ambiente de produção e servem para observar o comportamento com contrapressão, não para cravar um número de vazão.

Ao mudar a configuração do pipeline em `config/runtime.exs`, rode os testes de carga antes de publicar, mesmo que a mudança pareça pequena.

## Decisões em aberto

Há perguntas sem resposta fechada. Se as regras devem poder ser compostas com condições entre si ou se a lista ordenada basta. Se o cache de configuração deve ser compartilhado entre processadores em vez de cada um ter o seu. Se vale separar o pipeline por tamanho de evento, para que eventos pequenos nunca esperem atrás de um evento gigante. Nenhuma delas bloqueia o uso atual; registro aqui para não serem redescobertas do zero.

## Como mexer com segurança

Resumo prático para quem vai alterar algo. Leia `config/runtime.exs` antes. Lembre que o pipeline `Broadway` tem `6 processors` e que mudar isso afeta banco e ordem. Mantenha regras puras e idempotentes. Defina limite de tempo em qualquer regra que saia do processo. Grave antes de publicar. Se a mudança for no comportamento padrão do evento, avise os produtores, porque eles sentem a diferença na hora. E, se alguém falar em `bouncer7`, é o mesmo `moderation-gate`.
