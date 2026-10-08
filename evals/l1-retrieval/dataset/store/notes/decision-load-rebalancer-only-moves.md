---
id: 01KZABVR5KJXC1VR3CCN1Z2Q31
created: 2026-08-05T22:44-03:00
---

# load-rebalancer: só move cargas com status=PLANNED

Decisão tomada: o load-rebalancer só move cargas cujo status é `status=PLANNED`. Qualquer carga com outro status fica fora do rebalanceamento, mesmo quando o atraso é grande. O motivo é de custo: chamar de volta um caminhão que já saiu do pátio custa mais do que o atraso que se tentaria evitar. Esta nota existe para que ninguém reabra a discussão sem ver o raciocínio completo.

## Decisão em uma frase

O load-rebalancer considera para realocação apenas as cargas com `status=PLANNED`, porque recolher um caminhão que já saiu do pátio custa mais do que o atraso. Cargas que não estão em `status=PLANNED` são tratadas como fixas e entram no cálculo só como restrição, nunca como variável.

## Contexto

O FreightWeave planeja rotas multimodais, com trechos de caminhão e de trem, e rebalanceia as cargas quando acontecem atrasos. Quem usa são despachantes regionais de frete. Eles já convivem com atrasos de rotina, e o que mais reclamam é de mudança de plano depois que o motorista saiu. Um plano que muda com o caminhão na estrada gera ligação, retrabalho e, em geral, um trajeto vazio de volta.

## Por que o custo de recolher é maior

Quando o caminhão já saiu do pátio, trazê-lo de volta tem várias camadas de custo: combustível gasto duas vezes, tempo do motorista, janela de doca perdida no destino original e uma nova janela a negociar no pátio. Além disso, o trecho já percorrido não rende nada. O atraso, em contraste, costuma ser absorvido por folgas que o próprio plano já carrega. Na prática, esperar saiu mais barato do que desfazer a saída.

## O que significa estar em status=PLANNED

Uma carga em `status=PLANNED` tem rota e horário definidos, mas ainda não saiu. O veículo não partiu e nenhum trecho foi iniciado. Só nesse estado a troca de veículo, de trecho ou de horário é barata, porque nada físico precisa ser desfeito. Por isso o filtro usa exatamente esse valor e não uma lista de estados parecidos.

## O que fica de fora

Tudo que não for `status=PLANNED` não é movido. Isso inclui cargas já em trânsito, cargas já despachadas do pátio e cargas já concluídas. O rebalanceador não tenta uma exceção para atrasos longos. Se um despachante quiser mexer numa carga que já saiu, é decisão humana, fora do load-rebalancer.

## Como entra no modelo de otimização

O modelo no OR-Tools recebe só as cargas elegíveis como variáveis de decisão. As demais entram como dados fixos que ocupam capacidade e janelas de tempo. Assim o solver não gasta esforço em alternativas que a regra de negócio já proibiu, e o espaço de busca fica menor. Filtrar antes de montar o modelo é mais seguro do que penalizar movimentos depois, porque uma penalidade grande ainda pode ser vencida por outro termo da função objetivo.

## Onde o filtro é aplicado

O filtro vale na entrada do load-rebalancer, antes de qualquer cálculo. Não confiar em uma checagem posterior foi escolha deliberada: se o filtro ficasse só na saída, o solver poderia gerar planos inteiros que depois seriam descartados, e a mensagem para o despachante ficaria confusa. Com o filtro na entrada, o resultado que chega ao despachante já respeita a regra.

## Estado compartilhado no Redis

O status de cada carga é lido do estado mantido no Redis. A leitura precisa ser feita no momento do rebalanceamento, não de um cache velho. Uma carga que acabou de sair do pátio e ainda aparece como planejada é o risco principal desta decisão. Por isso o rebalanceador deve tratar a leitura do status como parte da mesma operação que decide o movimento, e preferir não mover quando houver dúvida.

## Eventos de atraso no Pub/Sub

Os atrasos chegam como eventos pelo Google Cloud Pub/Sub. Cada evento dispara uma tentativa de rebalanceamento. Eventos podem chegar fora de ordem ou repetidos, então o resultado não pode depender de uma ordem estrita. A regra de `status=PLANNED` ajuda aqui: se um evento antigo chegar depois da partida do caminhão, a carga já não será elegível e nada será movido.

## Interface FastAPI

A API em FastAPI expõe o resultado do rebalanceamento para os despachantes. Quando uma carga atrasada não pôde ser movida por não estar em `status=PLANNED`, a resposta deve dizer isso de forma clara, em vez de devolver uma lista vazia sem explicação. Quem lê precisa entender que a carga foi deixada de propósito, e não esquecida.

## Alternativas descartadas

A primeira alternativa era permitir mover cargas em trânsito com uma penalidade alta. Foi descartada porque a penalidade é difícil de calibrar e, em cenários extremos, o solver ainda escolhe recolher o caminhão. A segunda era deixar um limite de atraso decidir a exceção. Também foi descartada: um limite fixo cria comportamento diferente por região e é difícil de explicar ao despachante. A regra simples vence pela previsibilidade.

## Consequências aceitas

Aceitamos que algumas cargas sigam atrasadas quando uma realocação perfeita, no papel, seria possível. Aceitamos também que o rebalanceador às vezes ofereça menos opções do que o solver conseguiria achar. Em troca, o plano nunca manda um caminhão voltar para o pátio, e o despachante pode confiar que o que saiu continua como foi combinado.

## Riscos e pontos de atenção

O maior risco é a janela entre a saída real do caminhão e a atualização do status. Se a atualização atrasar, o rebalanceador pode mover uma carga que já partiu. Vale monitorar essa defasagem e, se for preciso, endurecer a leitura do status. Outro ponto: se alguém criar um novo status intermediário, ele não deve ser tratado como elegível sem nova decisão registrada.

## Quando revisitar

Revisitar só se o custo de recolher mudar de forma clara, por exemplo com pátios muito próximos entre si ou com mudança contratual que torne o atraso mais caro que a volta. Sem isso, a regra continua: o load-rebalancer só move cargas com `status=PLANNED`.
