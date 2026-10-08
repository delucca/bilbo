---
id: 01KE2D8KNRNNXVMZP3QZ36K422
created: 2026-01-03T14:07-03:00
---

# ladder-planner: alvo de qualidade sobe para VMAF 95

Esta nota substitui a nota anterior "ladder planner targets vmaf". O valor novo é este: o `ladder-planner` passa a mirar `VMAF score of 95`, no lugar do VMAF score de 93 que valia antes. A mudança veio depois de reclamações dos editores sobre banding (faixas visíveis em gradientes, céu, fundos escuros e fades) nas saídas que o alvo antigo considerava boas o bastante.

A decisão está tomada e não é um experimento. Quem for mexer no planejamento de ladders deve tratar o alvo novo como o ponto de partida e não reabrir a discussão sem dados novos dos editores. O que ainda está em aberto está na última seção.

## Contexto e motivo da mudança

O ReelForge recebe vídeo enviado por equipes de operações de mídia de publishers independentes, transcodifica em escadas de bitrate adaptativo e empacota em HLS. O `ladder-planner` é a peça que decide, para cada vídeo, quais degraus entram na escada e com que bitrate e resolução cada um sai. Ele não codifica nada. Ele calcula o plano, e o plano é executado depois pelas etapas de transcodificação com FFmpeg, orquestradas pelo AWS Step Functions, com os artefatos guardados no AWS S3.

O alvo antigo, VMAF 93, foi escolhido como um meio-termo razoável entre qualidade percebida e custo de banda e de armazenamento. Na época a escolha fazia sentido: a média das pontuações em conteúdo variado ficava confortável e quase ninguém reclamava. O problema é que a média escondia uma classe de conteúdo em que a métrica é otimista. Cenas com gradientes suaves, tons escuros e pouca textura recebem uma pontuação alta mesmo quando o olho humano vê degraus de cor. O VMAF é uma métrica de fidelidade em relação à fonte e tem boa correlação com a percepção na maior parte dos casos, mas banding em áreas planas é justamente onde a correlação enfraquece.

Os editores começaram a apontar o problema em revisões de material publicado. As queixas tinham um padrão fácil de reconhecer:

- o banding aparecia nos degraus intermediários e altos da escada, não só nos mais baixos, onde já se espera perda;
- era mais visível em telas grandes e em monitores calibrados, que é onde os editores revisam;
- aparecia em conteúdo com muita cena escura, créditos com fundo degradê, fades e céus abertos;
- o mesmo vídeo, reprocessado com mais bitrate, ficava limpo, o que apontava para falta de bitrate no plano e não para defeito da fonte.

O último ponto pesou na decisão. Se o defeito fosse da fonte, subir o alvo não ajudaria. Como o reprocessamento com mais bitrate resolvia, o caminho mais direto era subir o alvo de qualidade que o planejador persegue. Subir o alvo para `VMAF score of 95` empurra o plano para bitrates maiores justamente nos casos em que o banding aparece, porque são os casos em que a pontuação cai mais devagar com a redução de bitrate e, portanto, em que o planejador precisa de um piso mais exigente para parar de reduzir.

Alternativas que foram consideradas e deixadas de lado:

- Manter o alvo antigo e adicionar uma regra especial para conteúdo escuro ou com gradiente. Descartada porque exigiria um detector de conteúdo, mais uma parte móvel para manter, e a classificação erraria em casos de borda que os editores iriam notar do mesmo jeito.
- Trocar a métrica por outra mais sensível a banding. Descartada por ora porque o resto do pipeline, os relatórios e as comparações históricas estão todos em VMAF. Trocar de métrica quebraria a comparação com o que já foi publicado.
- Resolver só no codificador, com ajustes de dithering ou de parâmetros de quantização adaptativa no FFmpeg. Isso continua sendo uma melhoria possível e complementar, mas não substitui o alvo: sem um piso de qualidade mais alto no plano, o codificador recebe pouco bitrate para trabalhar.
- Subir o alvo só para alguns clientes. Descartada porque cria configuração por cliente que ninguém quer manter e porque o problema não é específico de um publisher.

A escolha final foi a mais simples: um alvo único e mais alto, aplicado pelo `ladder-planner` a todos os vídeos, com o valor antigo aposentado.

## Como o alvo entra no planejamento

Esta seção descreve o comportamento em termos gerais. Não está aqui o código nem os nomes de configuração, e quem for alterar deve ler o próprio `ladder-planner` em vez de confiar nesta descrição para detalhes de interface.

O planejador trabalha por vídeo. Ele recebe as características da fonte (resolução, taxa de quadros, duração, complexidade medida em análise prévia) e produz uma lista de degraus. Para cada degrau, ele procura o menor bitrate que ainda atinge o alvo de qualidade naquela resolução. A busca é feita sobre amostras de codificação de teste e pontuações VMAF calculadas contra a fonte redimensionada. O alvo é o critério de parada: o planejador reduz o bitrate enquanto a pontuação medida se mantém igual ou acima do alvo e escolhe o último ponto que ainda cumpre.

Com o alvo em `VMAF score of 95`, o efeito prático é que o ponto de parada se desloca para bitrates mais altos. O deslocamento não é uniforme:

- Em conteúdo simples e com muita textura, a curva de pontuação sobe rápido com o bitrate e a diferença entre o alvo antigo e o novo é pequena.
- Em conteúdo de gradientes suaves e cenas escuras, a curva é mais achatada perto do topo, então o mesmo ganho de pontuação custa bem mais bitrate. É aqui que o aumento se concentra, e é aqui que os editores reclamavam.
- Nos degraus de resolução mais baixa, o alvo pode ser inalcançável dentro dos limites de bitrate permitidos para aquele degrau. Nesses casos o planejador continua usando o teto do degrau e registra que o alvo não foi atingido, em vez de falhar o plano inteiro. Esse comportamento já existia com o alvo antigo e não mudou, mas agora ele deve ser disparado com mais frequência.

O plano gerado é entregue ao fluxo do Step Functions, que dispara as codificações com FFmpeg e depois o empacotamento HLS. Nada nas etapas seguintes precisa conhecer o alvo. Elas recebem bitrates e resoluções prontos. Isso foi intencional e é um motivo para manter o alvo concentrado no `ladder-planner`: a mudança de política de qualidade fica em um lugar só.

Outro ponto a lembrar é que o plano é gravado junto com os artefatos no S3. Planos antigos, calculados com o alvo anterior, continuam guardados como estavam. Eles não são recalculados sozinhos. Um vídeo já processado só passa a seguir o alvo novo se for reprocessado de propósito. Quem comparar uma saída antiga com uma nova do mesmo vídeo deve esperar bitrates diferentes e não tratar isso como bug.

Sobre a escada em si: a mudança de alvo não adiciona nem remove degraus por regra. Ela muda os bitrates dentro de cada degrau. Como o alvo mais alto pode fazer dois degraus vizinhos ficarem muito próximos em bitrate em certos vídeos, o planejador já tem a lógica de descartar degraus redundantes, e essa lógica continua valendo. Se a escada de algum vídeo ficar com menos degraus do que antes, é o comportamento esperado, não uma regressão.

## Custos e riscos

Subir o alvo não é de graça. Os custos são reais e foram aceitos de propósito, em troca de menos reclamação sobre banding.

Banda e entrega. Bitrates maiores significam mais bytes por segundo de reprodução. Para os publishers, isso aparece na conta de entrega e, para quem assiste em conexão fraca, o player do HLS vai cair para degraus mais baixos com mais frequência, porque os degraus altos ficaram mais pesados. A adaptação continua funcionando, mas a experiência em rede ruim pode ficar um pouco pior nos degraus intermediários do que era. Não há número fechado aqui porque o impacto depende muito do catálogo de cada publisher.

Armazenamento. Os segmentos e as variantes no S3 ficam maiores. O aumento é maior em catálogos cheios de conteúdo escuro ou suave e menor em catálogos de conteúdo movimentado. Quem administra o custo de armazenamento de um publisher deve esperar um aumento perceptível e acompanhar o crescimento nos primeiros ciclos de reprocessamento.

Tempo de processamento. A busca do planejador passa a convergir em pontos de bitrate mais altos, e codificar em bitrate mais alto costuma levar um pouco mais de tempo. Além disso, em casos em que o alvo é inalcançável no degrau, o planejador pode gastar mais amostras antes de desistir. O efeito total no tempo do fluxo é moderado e fica dentro do que o Step Functions já comporta, mas vale ficar de olho em vídeos longos.

Risco de falsa sensação de resolução. O alvo mais alto reduz o banding na maioria dos casos, mas não garante que ele desapareça. Como a métrica é otimista em áreas planas, ainda pode haver vídeos em que a pontuação passa do alvo e o banding continua visível. Se os editores voltarem a reclamar, a resposta provável não é subir o número de novo às cegas, e sim olhar a métrica e o codificador, como descrito na próxima seção. Subir o alvo indefinidamente tem retorno decrescente e custo crescente.

Risco de comparações históricas. Relatórios que comparam qualidade ao longo do tempo vão mostrar um degrau na data da mudança. Isso não é melhora ou piora do pipeline, é troca de política. Quem produz relatório para os publishers deve avisar disso para ninguém confundir.

Risco de inconsistência na migração. Enquanto o catálogo antigo não for reprocessado, haverá uma mistura de vídeos planejados com o alvo antigo e com o novo. Para o espectador isso só aparece se ele notar diferença de qualidade entre títulos. Para a operação, significa que uma queixa de banding num título antigo deve primeiro ser checada contra o alvo com que ele foi planejado. Se foi o antigo, a solução é reprocessar, e não investigar o planejador.

## O que verificar depois e o que fica em aberto

Para confirmar que a mudança está funcionando, o caminho é olhar resultados, não só configuração. Sugestões práticas:

- Pegar um conjunto pequeno de vídeos que antes geraram queixa de banding, reprocessar com o alvo novo e pedir que os mesmos editores comparem lado a lado com a versão antiga. A opinião deles é o critério real, porque foi a reclamação deles que motivou a mudança.
- Conferir nos registros do planejador quantos degraus terminaram com o alvo não atingido por causa do teto de bitrate. Se for uma fração grande em degraus que importam, o teto é que precisa de revisão, não o alvo.
- Comparar o tamanho total de saída de um lote representativo antes e depois, para ter o número de custo real do catálogo de cada publisher e não depender de estimativas gerais.
- Olhar o comportamento dos players em redes fracas, para ver se a queda para degraus baixos ficou mais frequente do que o aceitável.

Pontos que continuam em aberto e que podem virar nova decisão:

- Se vale combinar o alvo mais alto com ajustes de codificação no FFmpeg voltados a banding, como técnicas de dithering ou quantização adaptativa mais sensível a áreas planas. Seria uma melhoria complementar que pode permitir bitrates menores para o mesmo resultado visual.
- Se vale acrescentar uma métrica ou verificação auxiliar sensível a banding, usada só como alarme e não como critério de parada, sem trocar o VMAF como métrica principal.
- Se o catálogo antigo deve ser reprocessado em massa ou só sob demanda, quando um publisher reclamar. A inclinação atual é reprocessar sob demanda, porque o reprocessamento em massa tem custo alto de computação e de armazenamento e a maior parte do catálogo antigo não gerou queixa.
- Se o teto de bitrate de alguns degraus precisa subir para que o alvo seja atingível. Isso muda o custo de entrega e deve ser decidido com os publishers, não só pela equipe técnica.

Regras de bolso para quem pegar isso depois. Primeiro: se alguém perguntar qual é o alvo do `ladder-planner`, a resposta é `VMAF score of 95`, e o valor de 93 é histórico. Segundo: se uma saída tiver banding, descobrir com qual alvo ela foi planejada antes de qualquer outra coisa. Terceiro: não tratar diferença de bitrate entre saída antiga e nova do mesmo vídeo como defeito. Quarto: qualquer nova mudança de alvo deve vir acompanhada de uma checagem com editores em vídeos problemáticos conhecidos, porque foi assim que esta decisão foi justificada e é assim que ela deve ser revista.

Esta nota é a referência atual sobre o alvo de qualidade do `ladder-planner`. Se o valor mudar de novo, atualizar esta mesma nota em vez de criar outra, e manter aqui o histórico do que foi trocado e por quê.
