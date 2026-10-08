---
id: 01KCSP0YX5C1S97Y4W4S16MTZF
created: 2025-12-18T18:31-03:00
---

# Plano: delay-monitor com streaming pull

O delay-monitor vai deixar o pull síncrono e passar a usar streaming pull no Google Cloud Pub/Sub, com controle de fluxo definido em `max_messages=200`. Este plano registra o motivo, o que muda no código, a ordem de entrega, os riscos e como saber que deu certo. Ainda é plano: nada disso foi implementado quando escrevi.

O delay-monitor é o componente do FreightWeave que recebe os eventos de atraso (caminhão parado, trem retido, janela de terminal perdida) e avisa o planejador para rebalancear as cargas. Hoje ele busca mensagens em lotes, processa e confirma. O planejamento em si usa OR-Tools e fica fora do delay-monitor. O estado compartilhado fica no Redis, e a API que os despachantes usam é em FastAPI.

## Contexto e motivação

Com o pull síncrono, o delay-monitor pede um lote, espera a resposta, processa tudo e só então pede outro. Entre um pedido e outro há um intervalo em que nada chega. Em horário de pico, com muitos atrasos ao mesmo tempo, esse intervalo vira latência: o despachante vê o atraso na tela depois do que deveria, e o rebalanceamento começa tarde.

O segundo problema é a forma da carga. Os atrasos chegam em rajadas, por exemplo quando um pátio ferroviário fecha e dezenas de rotas são afetadas juntas. O laço síncrono ou fica ocioso, esperando, ou fica sobrecarregado, com um lote enorme na mão. Ele não se ajusta bem a nenhum dos dois casos.

O streaming pull mantém uma conexão aberta e o serviço empurra as mensagens conforme chegam. Isso reduz a espera entre mensagens e deixa o cliente da biblioteca cuidar da renovação dos prazos de confirmação. O custo é que precisamos limitar o que fica em memória, e para isso serve o controle de fluxo.

## Decisão central: streaming pull com flow control

A decisão é trocar o pull síncrono por streaming pull e configurar o controle de fluxo com `max_messages=200`. Isso significa que o delay-monitor aceita no máximo esse número de mensagens pendentes (entregues e ainda não confirmadas) ao mesmo tempo. Quando o limite é atingido, o cliente para de receber novas mensagens até que algumas sejam confirmadas ou recusadas.

O valor `max_messages=200` é o ponto de partida acordado. Ele não saiu de uma medição fina. Foi escolhido por ser grande o bastante para absorver uma rajada sem deixar o processamento ocioso e pequeno o bastante para que um travamento não deixe uma pilha enorme de mensagens sem confirmar. Se os testes de carga mostrarem outra coisa, o valor muda, mas a mudança deve ser registrada aqui com o motivo.

O controle de fluxo deve limitar também o volume em bytes, não só a contagem, porque mensagens de atraso com muitos trechos afetados são maiores que as simples. O limite de bytes fica como configuração separada e ainda sem valor decidido. Não vou inventar um número aqui.

## O que muda no código

O laço de busca atual é substituído por um assinante de streaming com uma função de retorno (callback) chamada para cada mensagem. A função de retorno precisa ser curta: valida o formato, grava o que for necessário e delega o trabalho pesado. Não deve chamar o solver do OR-Tools dentro dela.

Pontos concretos:

- A configuração do controle de fluxo vira um objeto único, lido do ambiente, com `max_messages=200` como padrão. Assim o valor pode ser mudado sem novo deploy do código.
- O processamento de cada mensagem passa para um pool de trabalho com tamanho limitado. O pool não pode ser maior do que o necessário para consumir o limite do controle de fluxo.
- A confirmação (ack) só acontece depois que o efeito da mensagem foi gravado no Redis. Recusa (nack) em erro transitório; erro permanente de formato vai para a fila de mensagens mortas.
- O encerramento limpo passa a existir de fato: ao receber o sinal de parada, o assinante deixa de aceitar novas mensagens, espera as pendentes terminarem dentro de um prazo e só então fecha.

O código antigo do pull síncrono fica atrás de uma opção de configuração até o fim da migração, para podermos voltar sem reverter commits.

## Idempotência e ordem

Streaming pull aumenta a chance de entrega repetida e de mensagens fora de ordem, principalmente quando o prazo de confirmação expira durante uma pausa. O pull síncrono já podia repetir, mas era menos frequente e mais fácil de ignorar.

O delay-monitor precisa tratar cada evento de atraso como idempotente. A ideia é guardar no Redis, por rota e por trecho, o último estado de atraso conhecido junto com um marcador de versão ou de horário do evento. Se chegar um evento mais antigo que o estado guardado, ele é confirmado e descartado sem efeito. Se chegar o mesmo evento duas vezes, o segundo não dispara novo rebalanceamento.

Não dependemos de ordem de entrega do Pub/Sub. Se no futuro for preciso ordem por rota, a opção é usar chave de ordenação na publicação, mas isso muda o produtor e tem custo de vazão. Fica fora deste plano.

## Interação com o Redis e com o planejador

O delay-monitor escreve no Redis o estado de atraso e publica o pedido de rebalanceamento. Com streaming, as escritas chegam em rajada. Precisamos garantir que o Redis aguente isso sem virar o gargalo no lugar do Pub/Sub.

O que fazer: agrupar escritas relacionadas à mesma rota, usar pipeline do cliente Redis quando houver várias chaves no mesmo evento e manter um tempo de expiração nas chaves de estado transitório. Também convém juntar (debounce) vários atrasos da mesma rota em pouco tempo antes de pedir o rebalanceamento, porque o solver é caro e rodar uma vez por mensagem numa rajada desperdiça trabalho.

O planejador continua sendo chamado do mesmo jeito. Não alteramos o contrato entre o delay-monitor e o planejador neste plano. Só muda a cadência com que os pedidos chegam, que tende a ser mais agrupada e mais rápida.

## Etapas de entrega

A ordem proposta, de modo que cada etapa possa ser entregue e revertida sozinha:

- Primeiro, tornar o tratamento de mensagem idempotente ainda sobre o pull síncrono. Isso é seguro hoje e é pré-requisito da troca.
- Depois, extrair o processamento de mensagem para uma função independente da forma de entrega, para que o pull síncrono e o streaming usem o mesmo código.
- Em seguida, implementar o assinante de streaming com `max_messages=200`, atrás da opção de configuração, desligado por padrão.
- Ligar em ambiente de teste com tráfego sintético de rajadas e comparar com o pull síncrono.
- Ligar em uma região pequena de produção, com um despachante parceiro avisado.
- Expandir para as demais regiões e remover o caminho antigo depois de um período sem incidentes.

Não pular a primeira etapa. Se a idempotência não estiver pronta, o streaming vai gerar efeitos duplicados que serão difíceis de explicar.

## Riscos e pontos de atenção

O principal risco é o ajuste do prazo de confirmação contra o tempo de processamento. Se o processamento de uma mensagem demorar mais que o prazo e a renovação automática não cobrir, a mensagem volta e é processada de novo. A idempotência protege dos efeitos, mas gasta capacidade.

Outro risco: `max_messages=200` combinado com um pool de trabalho pequeno pode deixar mensagens esperando dentro do cliente, com o relógio do prazo correndo. O pool e o limite precisam andar juntos. Se o pool for bem menor que o limite, as mensagens ficam retidas sem progresso.

Também há o risco de várias instâncias do delay-monitor disputando a mesma assinatura. Cada instância aplica o seu próprio limite, então o total pendente na assinatura é o limite vezes o número de instâncias. Isso precisa entrar na conta ao dimensionar o Redis e o planejador.

Por fim, reconexões do stream são normais e não devem gerar alarme por si só. O alarme deve olhar para o atraso das mensagens mais antigas, não para cada reconexão.

## Observabilidade e critérios de sucesso

Antes de ligar, precisamos de métricas para comparar. As que importam: idade da mensagem mais antiga não confirmada, número de mensagens pendentes no cliente (para ver se o limite está sendo atingido com frequência), tempo entre a publicação do evento de atraso e a atualização no Redis, taxa de nack, taxa de entregas repetidas descartadas pela idempotência e volume na fila de mensagens mortas.

O sucesso é o seguinte: o tempo entre o evento de atraso e o rebalanceamento iniciado cai de forma visível nos horários de pico, sem aumento de mensagens mortas e sem aumento de efeitos duplicados percebidos pelos despachantes. Se o limite do controle de fluxo for atingido o tempo todo, é sinal de que o processamento é o gargalo e o valor de `max_messages=200` sozinho não resolve; aumentar o limite só empurra o problema para a memória.

## Plano de reversão

A reversão é desligar a opção de configuração e reiniciar o delay-monitor, que volta ao pull síncrono. Como o tratamento já é idempotente desde a primeira etapa, as mensagens pendentes do streaming que voltarem para a fila serão reprocessadas sem dano.

A remoção do código antigo só acontece depois de um período sem precisar da reversão. Até lá os dois caminhos são mantidos e testados.

## Perguntas em aberto

- Qual limite em bytes usar no controle de fluxo, e se ele deve ser diferente por região.
- Se o valor `max_messages=200` serve para todas as regiões ou se as maiores, com mais rajadas, pedem outro.
- Qual janela de junção (debounce) por rota é aceitável para os despachantes sem atrasar demais o aviso.
- Se vale separar o delay-monitor em dois processos, um que só recebe e grava estado e outro que decide rebalancear, para isolar o custo do solver.
- Como testar rajadas realistas sem depender do emulador, que não reproduz bem o comportamento de reentrega.

Quando alguma dessas for respondida, atualizar esta nota em vez de criar outra sobre o mesmo assunto.
