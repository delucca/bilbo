---
id: 01JYJC9PF22FV4MDJNSYQNCYNC
created: 2025-06-24T22:47-03:00
---

# Decisão: ladder-planner mira VMAF por título em vez de ladder fixa

Decidimos que o `ladder-planner` não usa mais uma escada fixa de renditions para todo vídeo. Ele passa a mirar um VMAF score of 93 por título e monta a escada a partir disso. O motivo principal é banda: conteúdo simples (entrevista com fundo estático, slides, animação limpa) alcança a qualidade alvo com bitrates bem menores do que a escada fixa gastaria, e a economia aparece direto na conta de entrega das editoras independentes que usam o ReelForge. Esta nota registra a decisão, o porquê, o que foi descartado e o que fica em aberto, para que ninguém precise refazer a discussão.

## Contexto

O ReelForge recebe vídeo enviado pelo cliente, transcodifica em escadas de bitrate adaptativo e empacota em HLS para streaming. Os usuários são times de operações de mídia em editoras independentes. Eles não têm equipe grande de engenharia de vídeo e muitas vezes nem olham a escada: sobem o arquivo e esperam que o player toque bem em qualquer rede.

Antes, a escada era a mesma para todo título. Funcionava, era previsível e fácil de explicar. O problema é que o catálogo dessas editoras é muito heterogêneo. Tem gravação de painel com câmera parada, tem documentário com muita movimentação, tem esporte, tem animação. Uma escada que serve para o pior caso desperdiça bits no caso simples, e uma escada calibrada para o caso simples deixa o caso difícil com artefatos visíveis nos degraus de cima.

O `ladder-planner` é o componente que decide quais renditions existem para um título: resoluções, bitrates e quantas camadas. Ele roda antes da transcodificação pesada, dentro do fluxo orquestrado pelo AWS Step Functions, e entrega um plano que os passos seguintes (FFmpeg, empacotamento HLS, gravação no AWS S3) apenas executam.

## A decisão

O `ladder-planner` passa a trabalhar com um alvo de qualidade, não com uma tabela de bitrates. O alvo é um VMAF score of 93 por título. Para cada título, o planner procura, em cada resolução candidata, o menor bitrate que ainda atinge esse alvo, e monta a escada com o que sobra de útil.

Três pontos importantes:

- O alvo é por título, não por rendition global. Cada vídeo ganha sua própria escada.
- O alvo é um piso de qualidade percebida. Se uma rendition não chega lá dentro do teto de bitrate permitido, ela sai da escada ou é marcada como melhor esforço, conforme a regra de limite descrita mais abaixo.
- A motivação é economia de banda em conteúdo simples. Em conteúdo difícil a escada por título pode até ficar mais cara que a fixa, e isso é aceito: preferimos pagar o necessário para manter a qualidade do que entregar degraus ruins.

## Por que não manter a ladder fixa

A escada fixa tinha vantagens reais, e vale listar para lembrar o que estamos abrindo mão:

- Previsibilidade de custo e de armazenamento por título.
- Comportamento idêntico entre títulos, o que facilita suporte.
- Nenhuma etapa extra de análise antes da transcodificação.

O que pesou contra foi o desperdício sistemático. Em conteúdo simples, os degraus altos da escada fixa ficam acima do necessário para a qualidade percebida, e o espectador paga esse excesso em dados e a editora paga em saída de rede. Em conteúdo complexo acontece o oposto: o degrau nominalmente igual parece pior. A escada fixa erra para os dois lados, e erra mais justamente onde o catálogo dessas editoras tem volume, que é conteúdo falado e de baixo movimento.

Outro ponto: com métrica objetiva no centro, a conversa com o cliente muda de "qual bitrate você quer" para "qual qualidade você aceita". A segunda pergunta é a que o time de operações consegue responder.

## Por que VMAF e por que esse alvo

Escolhemos VMAF porque correlaciona melhor com a percepção humana do que métricas simples de sinal, e porque o FFmpeg já consegue calculá-lo no mesmo ambiente em que fazemos a transcodificação, sem ferramenta nova na imagem de execução. Isso reduz a superfície de manutenção.

O valor do alvo, um VMAF score of 93, foi escolhido como ponto em que, nas amostras que revisamos a olho, a diferença para a fonte deixa de ser notada em condições normais de visualização, e em que subir mais custa bitrate desproporcional. Não é um número sagrado. É um compromisso entre qualidade percebida e banda, e deve ser tratado como parâmetro de configuração, não como constante enterrada no código. Se uma editora quiser mais ou menos, a mudança tem que ser possível sem recompilar.

Limitações conhecidas da métrica que precisamos ter em mente:

- VMAF mede fidelidade à fonte. Se a fonte já é ruim, o alvo é atingido com facilidade e o resultado continua ruim em termos absolutos.
- Ela é sensível ao modelo usado e ao tamanho de tela assumido. Mudar o modelo muda o significado do alvo.
- Cenas curtas e difíceis dentro de um título longo e fácil podem ser diluídas na média.

## Como o planner chega na escada

O fluxo, em linhas gerais, sem entrar em detalhes que ainda podem mudar:

1. O planner recebe os metadados do título e acesso ao arquivo enviado no AWS S3.
2. Ele faz uma análise barata do conteúdo, com codificações de teste curtas e amostradas, para estimar a relação entre bitrate e qualidade em cada resolução candidata.
3. Com essas curvas, ele escolhe por resolução o menor bitrate que cumpre o alvo de VMAF.
4. Remove degraus redundantes: se duas renditions vizinhas ficam muito próximas em bitrate e em qualidade, mantém só uma.
5. Emite o plano estruturado que o Step Functions repassa aos passos de transcodificação e de empacotamento HLS.

A análise usa amostras, não o vídeo inteiro, justamente para o planejamento não custar quase tanto quanto a transcodificação final. A escolha das amostras importa: precisam cobrir trechos fáceis e difíceis. Esse é o ponto mais delicado do desenho e está listado nas questões em aberto.

## Limites e proteções

Qualidade por título sem limites vira risco de custo e de compatibilidade. Por isso o planner opera dentro de uma moldura:

- Há um teto de bitrate por rendition, para um título muito difícil não gerar uma rendition absurda que o player em rede média nunca vai escolher.
- Há um piso de camadas: mesmo conteúdo muito simples precisa de uma escada com opções suficientes para o player adaptar quando a rede oscila. Escada de uma camada só não é adaptativa.
- As resoluções candidatas vêm de uma lista permitida, para não gerar tamanhos estranhos que alguns dispositivos tratam mal.
- Se o alvo não é atingido em nenhuma configuração permitida para uma resolução, essa resolução fica de fora ou entra marcada como melhor esforço, e o plano registra o motivo.

Esses limites são configuração, como o alvo. A ideia é que o planner nunca produza algo que a escada fixa antiga não poderia servir em termos de compatibilidade HLS, apenas com bitrates diferentes.

## Impacto em armazenamento e processamento

A decisão troca previsibilidade por eficiência, e há custos do lado do processamento que precisam ser vistos com honestidade.

Na entrega, esperamos menos bytes por visualização em conteúdo simples, que é o ganho que motivou tudo. Em armazenamento, também deve haver redução nesses títulos, já que as renditions ficam menores. Em conteúdo difícil, o armazenamento pode subir.

No processamento, a análise por título adiciona uma etapa antes da transcodificação. Essa etapa gasta CPU e tempo de relógio. Aceitamos isso porque o custo é pago uma vez por título e a economia de banda se repete a cada visualização. Para títulos com poucas visualizações esperadas, o saldo pode ser neutro ou negativo; ainda não temos como saber de antemão quais são, então o comportamento padrão vale para todos.

No fluxo do Step Functions, a etapa nova aparece como um estado adicional antes do mapeamento paralelo das renditions. Como o plano agora varia por título, o número de ramos paralelos também varia, e o fluxo precisa aceitar isso sem presumir um conjunto fixo.

## Alternativas descartadas

Consideramos algumas saídas antes de fechar:

- Manter a escada fixa e só ajustar a tabela. Reduziria um pouco o desperdício, mas continuaria errando para os dois lados por conteúdo.
- Escadas por categoria de conteúdo, escolhidas pelo cliente no envio (por exemplo "fala", "esporte"). É simples, mas depende de classificação manual que as equipes de operações não vão manter com consistência, e a fronteira entre categorias é vaga.
- Otimização por cena, com bitrate variando dentro do título. Dá o melhor resultado teórico, porém complica muito o empacotamento e o comportamento do player. Fica como possível evolução, não como parte desta decisão.
- Alvo por bitrate médio em vez de métrica de qualidade. Não captura a diferença de dificuldade entre títulos, que é justamente o que queremos explorar.

## Riscos e questões em aberto

O que ainda pode dar errado, em ordem aproximada de preocupação:

- Amostragem ruim. Se as amostras da análise não representam o título, o plano fica otimista ou pessimista demais. Precisamos validar a estratégia de amostragem com títulos reais de várias editoras.
- Determinismo. A mesma entrada deve gerar o mesmo plano. Se a análise tem qualquer componente não determinístico, reprocessar um título pode mudar a escada, o que atrapalha cache e depuração. Vale tratar isso como requisito.
- Divergência entre a estimativa do planner e a qualidade real depois da transcodificação final. Convém medir o VMAF no resultado em uma amostra de títulos e comparar com o alvo, para saber o tamanho do erro.
- Comunicação com clientes que estavam acostumados com a escada fixa. A mudança de bitrates pode ser notada em painéis de monitoramento deles e gerar perguntas. Vale ter uma explicação curta pronta.
- Escolha do modelo de VMAF e da tela de referência. Foi adiado definir se haverá perfis diferentes para celular e TV.

## Como reverter

Se a abordagem se mostrar problemática, o caminho de volta é simples por desenho: o planner pode ser configurado para emitir a escada fixa antiga em vez de calcular uma. Como o restante do fluxo consome apenas o plano, nada a jusante precisa mudar. Manter esse modo de escada fixa funcionando é parte do compromisso desta decisão, não algo para apagar depois do lançamento. Também vale permitir a escolha por cliente, para que uma editora com problema volte à escada antiga sem afetar as outras.

## Próximos passos

- Fechar a estratégia de amostragem e testá-la em um conjunto variado de títulos, incluindo casos extremos de muito fácil e muito difícil.
- Expor alvo de qualidade, teto de bitrate, piso de camadas e lista de resoluções como configuração documentada.
- Medir a qualidade real das renditions geradas e comparar com o alvo de VMAF score of 93, registrando o erro típico.
- Acompanhar banda entregue e armazenamento antes e depois, separando conteúdo simples de conteúdo difícil, para confirmar que a economia que justificou a decisão existe de fato.
- Revisitar o alvo depois de algum tempo de produção, com dados das editoras. Se o valor mudar, atualizar esta nota em vez de criar outra.
