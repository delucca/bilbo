---
id: 01KK44TFZA8TABWVE9ZC3Z3X7K
created: 2026-03-07T09:37-03:00
---

# manifest-publisher: direção geral escolhida para publicação de manifestos

A equipe decidiu que o `manifest-publisher` vai ser o único ponto que decide quando um conjunto de renditions fica visível para o player. Nada mais no pipeline escreve manifesto HLS no lugar final. O transcodificador com FFmpeg produz segmentos e playlists de trabalho, o empacotamento junta tudo, e só o `manifest-publisher` promove o resultado para o que o público enxerga. Esta nota registra a direção, o motivo e o que ainda ficou solto. Não tem valores exatos aqui de propósito: o que importa é o desenho, e os parâmetros mudam conforme o cliente e o tipo de conteúdo.

A discussão veio de um incômodo antigo. Em mais de uma ocasião, o time de operações de mídia de uma editora viu o player carregar um manifesto que apontava para segmentos que ainda não estavam todos no armazenamento de objetos. O vídeo começava, rodava um tempo e travava numa rendition. Ninguém tinha errado de verdade: cada etapa fazia o seu trabalho, só que a ordem de visibilidade não era garantida por ninguém. A decisão tenta resolver isso na raiz, dando a responsabilidade a um componente só.

## O que foi decidido

A direção tem alguns pontos, todos gerais.

O `manifest-publisher` é o dono da visibilidade. Isso quer dizer que a presença de um manifesto no destino público é o sinal de que o conteúdo está pronto. Se o manifesto está lá, tudo o que ele referencia já está lá também e já foi conferido. Se algo falhou no meio, o manifesto simplesmente não aparece, e o conteúdo antigo, quando existe, continua valendo.

A publicação é feita em ordem fixa de dependência: primeiro as coisas que são referenciadas, depois as que referenciam. Segmentos e playlists de rendition entram antes, o manifesto mestre entra por último. Essa ordem não é negociável e não depende de quem chamou o componente.

O componente não transcodifica e não reempacota. Ele recebe como entrada uma descrição do que foi produzido, confere essa descrição contra o que existe de fato no armazenamento, monta ou valida o manifesto e publica. Se a descrição e a realidade divergirem, ele falha com uma mensagem clara em vez de tentar consertar sozinho. A tentação de deixar o publisher "dar um jeito" foi discutida e descartada, porque esconde defeitos de etapas anteriores e deixa o diagnóstico mais difícil depois.

A operação é idempotente. Rodar de novo para a mesma entrada leva ao mesmo estado final, sem duplicar nada e sem deixar o público num estado intermediário. Isso vale tanto para reexecução manual quanto para a repetição automática que a orquestração faz quando uma etapa falha por motivo transitório.

A troca de versão de um manifesto já publicado segue o mesmo princípio: a versão nova só substitui a antiga quando está completa e validada. Não existe janela em que o público vê uma mistura das duas.

## Por que essa direção e não outra

Foram consideradas algumas alternativas, e vale deixar escrito por que cada uma perdeu, porque a pergunta vai voltar.

A primeira alternativa era deixar cada etapa de empacotamento escrever o seu próprio pedaço no destino público, como já acontecia em parte. Era a mais simples de manter no curto prazo. Perdeu porque espalha a regra de ordem por vários lugares, e qualquer mudança no fluxo corria o risco de quebrar a garantia sem que ninguém percebesse. A regra "manifesto por último" precisa morar num lugar só para ser verificável.

A segunda alternativa era confiar na consistência do armazenamento e no tempo: publicar tudo junto e torcer para que a propagação fosse rápida o bastante. Isso funciona na maioria dos casos e falha justamente nos casos que geram chamado de suporte. Para um produto usado por equipes pequenas, sem ninguém de plantão olhando player, falha rara e silenciosa é pior que falha frequente e barulhenta. Preferimos que o erro apareça cedo e na etapa certa.

A terceira alternativa era um serviço de longa duração só para publicação, com estado próprio. Foi descartada por custo operacional. O `manifest-publisher` fica como uma unidade de trabalho curta, chamada pela orquestração do Step Functions, sem estado que sobreviva entre execuções. O estado de verdade fica no armazenamento e na própria definição da execução. Menos peça para operar, menos coisa para esquecer ligada.

A quarta alternativa era publicar manifestos parciais enquanto o transcode ainda roda, para o conteúdo começar a tocar mais cedo. É um recurso desejável para alguns clientes, e não foi rejeitado para sempre. Só ficou fora desta decisão. Se for feito, tem que ser uma modalidade explícita do próprio `manifest-publisher`, com regras próprias, e não um atalho de outras etapas. Ver a seção de pontos em aberto.

O fio comum de todas as escolhas é o mesmo: concentrar a garantia num componente pequeno, fácil de testar e fácil de explicar para quem opera. A equipe de mídia do cliente não lê código Rust. Ela precisa conseguir entender, numa frase, quando um vídeo aparece no ar.

## Como encaixa no fluxo

O fluxo geral continua como estava. O upload dispara a orquestração, o transcode gera a escada de bitrates com FFmpeg, o empacotamento organiza segmentos e playlists, e no fim entra o `manifest-publisher`. A mudança é que o fim do fluxo agora tem um contrato mais firme.

O contrato de entrada é uma descrição estruturada do que foi gerado: quais renditions existem, onde estão os artefatos de cada uma, quais faixas de áudio e legenda acompanham o vídeo e qual é a intenção de publicação. Essa descrição vem da etapa anterior, não é reconstruída olhando o armazenamento. A conferência existe para validar a descrição, não para substituí-la.

O contrato de saída é simples: ou o conteúdo ficou publicado e consistente, ou nada mudou para o público e a execução reporta falha com causa legível. Não há terceiro estado documentado. Se aparecer um caso que parece exigir um terceiro estado, isso deve ser tratado como bug de desenho e trazido para esta nota, não contornado no código.

Sobre a orquestração: as repetições automáticas ficam do lado do Step Functions, e o `manifest-publisher` assume que pode ser chamado várias vezes para a mesma entrada. Ele não implementa política própria de repetição escondida por dentro. Se uma chamada ao armazenamento falha, ele devolve o erro, classificado como transitório ou definitivo, e a orquestração decide o que fazer. Misturar as duas camadas de repetição já deu dor de cabeça em outros projetos, e aqui preferimos uma camada só.

Sobre HLS em particular: a regra é seguir o formato de modo conservador. Manifestos devem ser compatíveis com a maioria dos players usados pelos clientes, inclusive os mais velhos que ainda aparecem em televisores e aparelhos antigos. Recursos mais novos da especificação entram só quando há demanda real e teste em player de verdade. Isso significa que o publisher valida o manifesto contra um conjunto de regras internas de compatibilidade, e não apenas contra a especificação no papel.

A ordenação das renditions dentro do manifesto mestre segue um critério estável e previsível, para que republicar o mesmo conteúdo gere o mesmo arquivo. Isso ajuda a comparar versões, ajuda cache e ajuda a perceber mudança acidental. Estabilidade de saída é parte do contrato.

Nomes e localização dos artefatos públicos seguem a convenção já adotada no projeto. Esta nota não repete essa convenção; quem precisar deve olhar o código do componente e a documentação de layout. A decisão aqui é só que o publisher é quem aplica a convenção na hora de promover, e que nenhuma outra etapa grava no destino público.

## Cuidados e riscos que a equipe aceitou

Concentrar a responsabilidade tem preço, e a equipe sabe qual é.

O `manifest-publisher` vira ponto único de falha lógica. Se ele tiver um bug, todo o conteúdo novo fica retido, ou pior, publicado errado. Para compensar, o componente deve ser pequeno, ter testes que cubram os casos ruins com mais cuidado que os casos bons, e ser a parte do pipeline onde mudanças passam por revisão mais atenta. A regra prática: mexeu no publisher, pede segunda leitura de quem não escreveu.

Existe o risco de a validação ser rígida demais e travar conteúdo legítimo. Já aconteceu em projetos parecidos de a checagem rejeitar algo que o player aceitaria sem problema. A orientação é começar rígido nas regras que protegem o espectador de falha visível, como segmento ausente ou referência quebrada, e mais tolerante nas regras estéticas. Quando uma regra rígida bloquear conteúdo bom, o ajuste deve ser feito na regra, com registro do motivo, e não com um desvio manual que fique escondido.

Há o risco de cache. Manifestos publicados passam por camadas de distribuição, e uma substituição de versão pode ficar visível em momentos diferentes para espectadores diferentes. O componente não promete que todo mundo vê a versão nova no mesmo instante. Promete que, em qualquer instante, o que cada um vê é um conjunto consistente, velho ou novo. Os clientes precisam ouvir essa diferença com clareza, porque a expectativa errada gera chamado. A equipe de produto ficou de reforçar isso na documentação voltada a operações.

Há o risco de execuções concorrentes sobre o mesmo conteúdo, por exemplo um reprocessamento manual disparado enquanto a execução original ainda não terminou. A direção é que o publisher trate a publicação como seção crítica por conteúdo, de forma que duas execuções não se atropelem, e que a segunda espere ou falhe de modo explícito. Como exatamente isso é garantido, se por trava no armazenamento, por condição na orquestração ou por outro mecanismo, é detalhe de implementação e pode mudar. A garantia é o que está decidido.

Há também o risco humano: alguém com pressa publicar um manifesto à mão no destino público para destravar um cliente. Acontece, e vai acontecer de novo. Não vamos proibir tecnicamente, mas a prática combinada é que qualquer ajuste manual seja seguido de uma passada do `manifest-publisher` para reconciliar o estado, e que o ajuste seja anotado. Estado público que o publisher não conhece é o tipo de coisa que volta a morder meses depois.

Por fim, o risco de custo. Conferir artefatos no armazenamento antes de publicar gera chamadas extras. A equipe considerou esse custo aceitável diante do problema que ele evita, mas ficou combinado olhar o volume quando a escala crescer. Se a conferência virar gargalo, a solução esperada é conferir de modo mais inteligente, usando a descrição de entrada e amostragem onde fizer sentido, e não remover a conferência.

## Operação e diagnóstico

Para quem opera, a decisão tem consequências práticas, todas gerais.

Quando um conteúdo não aparece, o primeiro lugar a olhar é o resultado da execução do `manifest-publisher`. Se ele falhou, a causa deve estar na mensagem. Se ele nem foi chamado, o problema está antes, no transcode ou no empacotamento. Essa divisão simples é um dos ganhos principais da decisão: o diagnóstico começa por uma pergunta só, "o publisher rodou e o que ele disse?".

As mensagens de falha devem falar a língua de quem opera, não a de quem programa. Dizer qual rendition, qual artefato e qual tipo de problema, com sugestão do que olhar. Evitar despejar erro cru de biblioteca sem contexto. O detalhe técnico completo vai para o registro, a frase legível vai para quem está olhando o painel.

O componente deve deixar rastro suficiente para reconstruir o que ele fez: o que recebeu, o que conferiu, o que publicou e o que substituiu. Esse rastro serve para auditoria, para suporte e para investigar reclamação de espectador depois do fato. Não é preciso guardar para sempre, mas precisa durar o tempo em que reclamações costumam chegar.

Reversão é um caso de uso esperado, não exceção. Se uma versão publicada se mostra ruim, deve ser possível voltar à anterior pelo mesmo caminho do publisher, sem edição manual. Para isso o componente precisa saber distinguir versões e não destruir a anterior imediatamente após promover a nova. Por quanto tempo guardar versões antigas é uma decisão de retenção que depende do cliente e fica fora desta nota.

Testes: a direção é testar o `manifest-publisher` principalmente com entradas fabricadas que reproduzem falhas reais já vistas, como artefato ausente, descrição incompleta, repetição de chamada e concorrência. Teste com mídia real continua existindo, mas na camada de cima, no fluxo completo. A razão é velocidade e clareza: um teste de publisher que depende de transcodificar vídeo de verdade é lento e falha por motivos que não são do publisher.

Observabilidade: o componente deve emitir sinais que permitam ver tendência, por exemplo quantas publicações falham e por qual categoria de causa. Isso ajuda a perceber degradação lenta, como um tipo de entrada que passa a falhar mais, antes de virar reclamação. Os detalhes de métricas ficam com quem cuida da operação.

## Pontos em aberto

O que não foi decidido, para ninguém tratar como resolvido:

Publicação parcial ou progressiva, para o conteúdo começar a tocar antes do transcode terminar. Há interesse, principalmente de publishers com conteúdo noticioso. Precisa de desenho próprio, porque quebra a ideia simples de que manifesto presente significa tudo pronto. Se vier, deve ser uma modalidade nomeada e opt-in no `manifest-publisher`, com semântica descrita à parte.

Política de retenção de versões antigas de manifesto e de segmentos órfãos. Hoje depende de decisão caso a caso. Falta uma regra geral que equilibre custo de armazenamento e capacidade de reverter.

Como expor o estado de publicação para a interface que os clientes usam. A decisão aqui garante que o estado existe e é confiável; como mostrar isso para quem não é técnico é outro assunto, com outro dono.

Suporte a recursos mais novos de HLS. A postura conservadora vale por enquanto. Quando algum cliente depender de um recurso novo, avaliar com player real antes de liberar, e registrar a decisão numa nota própria.

Relação com outros formatos de empacotamento, caso o produto passe a oferecer. A expectativa é que o princípio se mantenha: um componente dono da visibilidade, com ordem de dependência respeitada. Se for um componente novo ou o mesmo estendido, fica para quando o requisito aparecer.

## Como usar esta nota

Se você é uma pessoa ou um agente prestes a mexer no `manifest-publisher`, a pergunta de controle é: minha mudança preserva a ideia de que o público só vê conteúdo completo e consistente, e de que só este componente decide isso? Se sim, siga. Se a mudança exige que outra etapa escreva no destino público, ou que o publisher aceite entrada incompleta em silêncio, pare e traga a discussão de volta antes de implementar.

Se a decisão mudar, atualize esta nota em vez de criar outra, e apague o que deixou de valer. Nota velha dizendo uma coisa enquanto o código faz outra é pior que nota nenhuma.
