---
id: 01KKNREH5EWHY5CS417Y06SDPX
created: 2026-03-14T05:47-03:00
---

# instrument-ingest-worker: prefetch do RabbitMQ reduzido para 20

Decisão: o instrument-ingest-worker passa a consumir da fila do RabbitMQ com um `prefetch count of 20` em vez de 100. O motivo é simples: arquivos grandes de espectros esgotavam a memória do worker quando o prefetch estava em 100. Com 100 mensagens em voo por consumidor, o processo acumulava muitos arquivos grandes ao mesmo tempo e acabava morto por falta de memória. Com `prefetch count of 20` o consumo de memória fica previsível e o worker para de cair. Este texto foi escrito para que quem ler só ele consiga responder: qual é o valor, qual era o anterior, e por que mudamos.

## Resumo da decisão

O valor anterior era 100. O valor novo é 20, e é o que vale hoje para o instrument-ingest-worker. A mudança é só de configuração do consumidor (QoS do canal no RabbitMQ). Não mexemos no formato das mensagens, nem no contrato com os instrumentos, nem no esquema do SQL Server.

O ponto central: o prefetch limita quantas mensagens não confirmadas o broker entrega ao consumidor de uma vez. Quando cada mensagem carrega, ou aponta para, um arquivo de espectro grande, o custo de memória por mensagem não é pequeno nem uniforme. Um número que funcionava bem para mensagens leves ficou alto demais para espectros pesados.

Quem for mexer nisso de novo deve lembrar que o número 20 não saiu de uma fórmula exata. Foi o valor que deu folga de memória sem derrubar a vazão de forma perceptível. Se alguém quiser trocar, precisa medir com arquivos grandes de verdade, não com amostras pequenas.

## Contexto do problema

O instrument-ingest-worker recebe mensagens com a notícia de que um instrumento terminou de gerar uma saída. Ele baixa ou lê o conteúdo, normaliza, grava metadados no SQL Server, guarda o arquivo bruto no Azure Blob Storage e deixa o rastro de auditoria pronto para o caderno eletrônico. Cientistas e o pessoal de conformidade dependem desse rastro, então perder mensagem ou processar pela metade não é aceitável.

Com o prefetch em 100, o comportamento em produção era assim: em dias normais, com arquivos pequenos, tudo corria bem. Quando chegava uma rajada de espectros grandes, por exemplo depois de uma corrida longa de um espectrômetro, o worker puxava muitas mensagens de uma vez, carregava muita coisa na memória e o processo era encerrado pelo sistema operacional ou pelo orquestrador. A mensagem não confirmada voltava para a fila, outro worker pegava o mesmo lote, e o ciclo podia se repetir.

O sintoma visível era reinício frequente do worker, atraso na chegada dos dados ao caderno e alertas de memória. O sintoma pior, para conformidade, era a dúvida sobre se algum arquivo tinha sido processado duas vezes ou deixado em estado parcial. A reentrega é segura se o processamento for idempotente, mas ninguém gosta de depender disso em ciclo de queda.

## Por que a memória estourava

Espectros brutos podem ser bem maiores que o resto do tráfego do sistema. O worker, na forma como foi escrito, mantém o conteúdo do arquivo em memória durante a validação e a conversão antes de enviar ao Blob Storage. Isso não é streaming puro. Então cada mensagem em andamento custa, no pior caso, o tamanho do arquivo mais as estruturas intermediárias da conversão.

Com o prefetch alto, o broker entrega mensagens além do que o worker consegue processar ao mesmo tempo. Elas ficam na memória do cliente, já entregues, esperando vez. Se o conteúdo for carregado cedo, o custo soma. Mesmo quando só o cabeçalho da mensagem fica em memória, o paralelismo interno do worker puxa vários arquivos grandes ao mesmo tempo, e o pico é a soma dos maiores.

Outro efeito: o coletor de lixo do .NET tem trabalho extra com objetos grandes, que vão para o heap de objetos grandes. Esse heap é coletado de forma mais cara e pode fragmentar. Com muitos buffers grandes vivos e liberados em ritmo irregular, a pressão de memória sobe mais do que a conta simples sugere. Reduzir o número de itens em voo ajuda a manter esse heap sob controle.

## Alternativas consideradas

Primeiro, manter o prefetch em 100 e aumentar a memória do contêiner. Descartamos porque só adia o problema: um lote de espectros ainda maiores estouraria de novo, e o custo de infraestrutura cresceria para atender um pico raro.

Segundo, trocar o processamento para streaming completo, sem carregar o arquivo inteiro. É o melhor caminho de longo prazo, mas é uma mudança maior no código, com risco para a integridade dos dados e para o cálculo de somas de verificação usadas na auditoria. Fica como trabalho futuro, não como correção imediata.

Terceiro, prefetch dinâmico, ajustado conforme o tamanho das mensagens. Dá certo em teoria, mas o broker não sabe o tamanho do arquivo apontado, e a lógica de adaptação adicionaria complexidade e novos modos de falha. Preferimos um número fixo e conservador.

Quarto, filas separadas para arquivos grandes e pequenos, cada uma com seu próprio prefetch. Também é uma boa ideia futura. Hoje exige que o produtor classifique a mensagem, e isso mexe em quem publica. Por enquanto, um único valor menor resolve.

A escolha final foi reduzir o prefetch para 20. É a mudança de menor risco, reversível em minutos, e resolveu o problema observado.

## Efeitos esperados e custos

O efeito principal é menos memória de pico por instância. O worker passa a manter menos arquivos grandes ao mesmo tempo, e o risco de ser encerrado por falta de memória cai bastante.

O custo é a vazão potencialmente menor em cenários de muitas mensagens pequenas. Com menos mensagens em voo, a latência da rede entre broker e consumidor pesa mais, porque o worker pode ficar esperando a próxima entrega. Na prática, para o perfil do nosso tráfego, essa perda não apareceu de forma relevante, porque o gargalo real está no armazenamento e na gravação no SQL Server, não na entrega do broker.

Se a fila crescer e o atraso incomodar, a resposta correta é adicionar instâncias do worker, não subir o prefetch. Mais instâncias com prefetch baixo escalam melhor e mantêm o teto de memória por processo.

Outro efeito colateral: a distribuição de carga entre instâncias fica mais justa. Com prefetch alto, uma instância podia acumular muitas mensagens pesadas enquanto outras ficavam ociosas. Com `prefetch count of 20` o broker redistribui com mais frequência.

## Como o valor é aplicado

O valor é definido na configuração do consumidor, na chamada de QoS do canal, e é aplicado por consumidor, não globalmente por conexão. Quem lê o código deve procurar onde o canal é aberto no instrument-ingest-worker e onde o limite de prefetch é passado. Evite espalhar o número em vários lugares: ele deve vir de uma única opção de configuração, com nome claro, para que a mudança futura seja um ajuste e não uma caça.

Cuidado com a diferença entre prefetch por consumidor e por canal no RabbitMQ. Se o worker abrir mais de um consumidor no mesmo canal, a interpretação do limite muda conforme a bandeira usada na chamada. Confira isso ao revisar, porque um valor lido como global pode dar um teto total muito diferente do esperado.

A confirmação de mensagens deve continuar manual e só acontecer depois que o arquivo estiver gravado no Blob Storage e os metadados estiverem confirmados no SQL Server. O prefetch baixo não substitui essa regra; ele só limita quanto trabalho não confirmado existe em um dado momento. Se o worker cair, no máximo esse lote pequeno volta para a fila.

## Riscos e pontos de atenção

Reentrega continua possível. Qualquer queda entre a gravação e a confirmação faz a mensagem voltar. O processamento precisa ser idempotente: a mesma saída de instrumento não pode gerar duas entradas de auditoria conflitantes nem dois registros duplicados. Isso já era verdade antes da mudança, e o prefetch menor só reduz o tamanho do lote afetado.

Se algum dia aparecer uma mensagem que sozinha estoura a memória, o prefetch não resolve. Nesse caso a mensagem fica voltando e deve ir para uma fila de mensagens mortas depois de um número limitado de tentativas. Vale conferir se essa política existe e está ativa; sem ela, uma mensagem venenosa pode travar um consumidor para sempre.

Ao subir o prefetch de novo, o primeiro sintoma a vigiar é o uso de memória do processo sob rajada de espectros grandes, não a média do dia. Os testes com arquivos pequenos enganam.

Também fique atento a mudanças de formato dos instrumentos. Um equipamento novo, ou uma configuração de aquisição com resolução maior, pode produzir arquivos bem maiores que os de hoje, e o valor de 20 pode deixar de ser suficiente. Nesse caso, o caminho é streaming, ou prefetch ainda menor, antes de aumentar memória.

## Como verificar que está funcionando

Olhe a memória residente do processo do instrument-ingest-worker durante uma rajada de arquivos grandes. O gráfico deve subir até um teto estável e não crescer sem parar. Olhe também o número de reinícios do worker: depois da mudança deve ficar perto de zero por causa de memória.

Na interface de gerenciamento do RabbitMQ, observe a quantidade de mensagens não confirmadas por consumidor. Ela não deve passar do valor configurado. Se passar, o QoS não está sendo aplicado como se pensa, e vale rever se a configuração chegou ao canal certo.

Observe o tempo entre a geração do arquivo no instrumento e a aparição da entrada no caderno. Se esse tempo piorar de forma clara só com a mudança, reavalie. Mas antes de mexer no prefetch, verifique se o gargalo não é o armazenamento ou o banco.

Para a conformidade, confira por amostragem que cada saída de instrumento tem exatamente um rastro de auditoria coerente, sem duplicata, mesmo após um reinício forçado do worker em teste.

## Quando revisar esta decisão

Revise se acontecer qualquer um destes casos: o worker passar a processar em streaming de fato, sem manter o arquivo inteiro em memória; as filas forem separadas por tamanho de arquivo; o perfil de memória do contêiner mudar muito; ou a vazão virar problema real e adicionar instâncias não resolver.

Se o streaming entrar, o prefetch pode voltar a subir, porque o custo de memória por mensagem passa a ser pequeno e constante. Nesse caso, registre uma nova decisão em vez de editar esta sem rastro, e diga qual medição justificou o novo valor.

Enquanto nada disso acontecer, o valor combinado é `prefetch count of 20` para o instrument-ingest-worker, e a razão dele é a memória esgotada por arquivos grandes de espectros quando o valor era 100.

## Resumo rápido para quem tem pressa

O instrument-ingest-worker consome do RabbitMQ com `prefetch count of 20`. Antes era 100. Mudamos porque arquivos grandes de espectros esgotavam a memória do worker. Para mais vazão, adicione instâncias; não aumente o prefetch sem medir com arquivos grandes. A confirmação de mensagem só ocorre depois de gravar no Blob Storage e no SQL Server, e o processamento deve seguir idempotente. O trabalho futuro mais promissor é streaming e filas separadas por tamanho, e só então faz sentido rever o número.
