---
id: 01KS4FSETZ4D3R00D9AEQSCJKS
created: 2026-05-21T02:23-03:00
---

# Direção geral para progress_events

Anotação rápida sobre o rumo que o time escolheu para `progress_events`. Não é spec e não traz valores. É para quem voltar ao assunto saber por que as coisas estão como estão e não refazer a discussão do zero.

A decisão central: `progress_events` passa a ser tratado como um registro de fatos, que só cresce, e não como uma tabela de estado que a gente vai corrigindo. Cada evento diz que alguma coisa aconteceu com um aluno em relação a um padrão curricular, num certo momento, e fica assim. O que o professor vê no painel (nível de domínio, lacunas, próximos exercícios) é calculado a partir desses fatos. Quando a conta muda, a gente recalcula. Os fatos continuam lá.

Isso parece óbvio escrito assim, mas a discussão foi longa, porque o caminho mais fácil era guardar só o estado atual por aluno e por padrão e atualizar no lugar. Já tínhamos feito algo parecido antes e deu problema toda vez que alguém perguntou "por que o aluno aparece com esse domínio?". Sem histórico não dá para responder.

## Contexto e por que decidir agora

O produto acompanha o progresso de estudantes do ensino médio contra padrões curriculares e sugere os próximos exercícios. Professores usam no dia a dia, normalmente entre uma aula e outra, com pouco tempo e pouca paciência para explicações longas. Se o sistema sugere um exercício estranho, a pergunta imediata é "por quê?". A resposta tem que sair de algum lugar, e esse lugar é `progress_events`.

Havia três pressões ao mesmo tempo. Primeiro, o modelo de sugestão em scikit-learn estava ficando mais exigente com os dados de entrada e precisava de sequências, não de fotografias. Segundo, a busca em Elasticsearch no painel estava lendo dados derivados de formas diferentes em lugares diferentes, e os resultados divergiam. Terceiro, as correções manuais feitas por professores (mudar uma avaliação, desfazer um lançamento errado) estavam sobrescrevendo informação sem deixar rastro.

Juntando as três, ficou claro que continuar com estado mutável era empurrar o problema. Por isso a decisão foi tomada agora, antes de o volume e o número de consumidores crescerem mais.

Também pesou o fato de que o desenho antigo misturava duas coisas: o que aconteceu e o que a gente acha que isso significa. Eram colunas na mesma linha. Quando a interpretação mudava (ajuste de peso, mudança de regra de domínio), os dados originais já tinham sido perdidos ou misturados. Separar fato de interpretação é o ponto que mais convenceu o time.

## O que foi escolhido

A direção, em linhas gerais:

- `progress_events` é append-only na prática. Eventos não são editados depois de gravados. Correção é um novo evento que referencia o anterior e o anula ou o substitui.
- Cada evento carrega o mínimo para ser entendido sozinho: quem, qual padrão, que tipo de acontecimento, a origem (exercício, avaliação, lançamento manual, importação) e o momento. Detalhes extras ficam num bloco livre e opcional, que não é consultado em filtros.
- O tipo do evento é um vocabulário fechado e pequeno, mantido no código Django. Adicionar tipo novo exige revisão. Preferimos poucos tipos bem definidos a muitos tipos quase iguais.
- Tudo que é derivado (domínio por padrão, resumo por turma, lista de lacunas) vive fora de `progress_events`, em estruturas próprias, e pode ser jogado fora e reconstruído.
- A escrita de eventos é síncrona e simples dentro da requisição Django. O processamento pesado depois dela é assíncrono, via Celery.
- A leitura para o painel passa pelos derivados, nunca por varredura direta dos eventos, exceto em telas de auditoria e de explicação.

Nada disso depende de número mágico. Quando houver ajuste fino (tamanho de lote, janelas de tempo, frequência de recálculo), isso vai para configuração e para outras notas, não para esta.

## Como os consumidores devem usar

Quem consome `progress_events` hoje são basicamente quatro partes: o cálculo de domínio, o módulo de sugestão de exercícios, a indexação para busca e as telas de explicação no Vue.js. A regra geral é que cada uma lê eventos por um caminho previsto e mantém seu próprio ponto de avanço, de forma que possa ser refeita sem afetar as outras.

O cálculo de domínio lê os eventos em ordem e produz o estado por aluno e padrão. Ele precisa ser determinístico: com os mesmos eventos e a mesma versão das regras, o mesmo resultado. Se alguém encontrar uma divergência entre duas execuções, isso é bug, não variação aceitável.

O módulo de sugestão usa os derivados como base e, quando precisa de sequência, lê eventos recentes de um recorte pequeno. Não deve montar sua própria visão do histórico por fora. Já houve tentativa de copiar eventos para uma tabela auxiliar do modelo; a decisão é não repetir isso, porque a cópia envelhece e ninguém lembra de quem é a responsabilidade.

A indexação em Elasticsearch trata o índice como projeção descartável. Se o índice estiver errado, reconstrói-se a partir dos eventos. Ninguém deve corrigir documento no índice na mão. Isso vale também em emergência: corrige-se a origem e reprocessa-se.

As telas de explicação mostram ao professor, em linguagem simples, os eventos que levaram a um estado. Elas leem pela API, que devolve eventos já com texto legível e sem expor campos internos. O front não interpreta tipos de evento por conta própria; a API entrega o que mostrar.

## Correções, anulações e dados ruins

Esse foi o ponto mais debatido e vale registrar o raciocínio. Professores erram lançamento, importações vêm com problema, e às vezes o próprio sistema grava algo que depois se mostra errado. O desenho anterior permitia editar a linha. O novo não permite.

A escolha foi: correção é evento. Um evento de anulação aponta para o evento original e diz que ele não deve mais contar. Se houver valor correto, grava-se um evento novo. O cálculo de domínio ignora o que foi anulado, mas a auditoria continua mostrando tudo, com a marcação de que foi anulado e por quem.

Vantagens que pesaram: dá para reconstruir o que o professor via em qualquer momento do passado; dá para responder reclamação de aluno ou de família com base no que realmente foi registrado; e o recálculo fica seguro, porque a entrada nunca muda por baixo.

Custos aceitos: o volume cresce mais do que com atualização no lugar; consultas de auditoria ficam um pouco mais trabalhosas; e é preciso disciplina para não "resolver rápido" com um update direto no banco. Combinamos que update direto em `progress_events` só acontece em situação excepcional, com registro do motivo e com duas pessoas cientes. Se virar rotina, a decisão está falhando e precisa ser revista.

Para dados ruins vindos de importação, a direção é isolar. Importações entram marcadas com a origem e com um identificador do lote, de forma que um lote inteiro possa ser anulado de uma vez se necessário. Não se tenta limpar no meio do caminho silenciosamente; se uma linha não passa na validação, ela vai para um relatório de rejeitados que o responsável pela escola consegue ver.

## Processamento assíncrono e consistência

A gravação do evento e a atualização dos derivados não acontecem na mesma transação. Aceitamos consistência eventual entre as duas. O professor pode lançar algo e ver o painel atualizar um instante depois. Para não confundir, a interface sinaliza quando há processamento pendente para aquele aluno, em vez de mostrar um valor antigo como se fosse atual.

As tarefas Celery que consomem eventos seguem algumas regras gerais:

- Devem ser idempotentes. Rodar duas vezes sobre o mesmo evento não pode mudar o resultado.
- Devem tolerar ordem imperfeita de chegada, mas o cálculo final usa a ordem lógica dos eventos, não a ordem em que a tarefa os viu.
- Falhas devem ser visíveis. Tarefa que engole erro e segue em frente é pior do que tarefa que para e avisa.
- Reprocessamento completo precisa ser possível sem parar o sistema, mesmo que lento. Se um dia a gente não conseguir reconstruir os derivados, perdemos a principal vantagem desta decisão.

Para o futuro, a ideia é que cada consumidor guarde até onde já leu e que esse marcador possa ser reposicionado manualmente. Os detalhes de como isso é armazenado ficam para quando implementarmos; o que está decidido é o princípio.

Ainda não decidimos como lidar com eventos que chegam muito atrasados, por exemplo notas lançadas bem depois do período. A tendência é aceitá-los e recalcular, mas isso afeta relatórios já entregues. Fica como ponto aberto.

## Privacidade, retenção e acesso

Os eventos falam de menores de idade, então o cuidado é maior do que num sistema comum. A direção geral: guardar o necessário para o propósito pedagógico e nada além. O bloco livre de detalhes não deve receber texto pessoal digitado por professores sem necessidade; comentários longos ficam em outro lugar, com regras próprias.

Acesso a `progress_events` é por escola e por turma, respeitando o vínculo do professor. A camada Django aplica o filtro; não confiamos no front para isso. Telas de auditoria têm perfil de acesso mais restrito.

Sobre retenção, a decisão é tratar o histórico como ativo, mas com política explícita de quanto tempo guardar e de como atender pedidos de remoção. Como o registro é append-only, remoção precisa ser planejada: o caminho aceito é remover ou anonimizar por processo próprio e controlado, e depois reconstruir os derivados. Isso não deve ser feito por improviso. Os prazos concretos pertencem à parte jurídica e de produto, e não ficam registrados aqui.

Também ficou combinado que dados enviados ao modelo de sugestão devem ser os derivados e recortes mínimos, sem identificação direta quando possível. Se o treinamento precisar de mais, passa por revisão.

## Alternativas descartadas e pontos abertos

Descartamos, em resumo, três caminhos.

Manter só o estado atual e atualizar no lugar. Simples, mas sem explicação e sem recálculo seguro. Foi o desenho que já nos causou dor.

Guardar estado e histórico em paralelo, ambos como fonte de verdade. Parecia um meio-termo, mas cria dois lugares que podem discordar, e ninguém saberia qual vale. Preferimos uma fonte só e o resto derivado.

Usar o Elasticsearch como armazenamento principal dos eventos. Tentador pela busca, mas não queremos que a fonte de verdade dependa de um índice pensado para consulta. O índice continua sendo projeção.

Pontos abertos, sem resposta ainda:

- Como tratar eventos muito atrasados e seu efeito em relatórios já emitidos.
- Se e quando compactar ou arquivar eventos antigos, e como manter a explicação disponível depois disso.
- Como versionar as regras de domínio de modo que se possa dizer com qual regra um estado foi calculado.
- Como expor ao professor a diferença entre "o sistema recalculou" e "o aluno mudou de fato", para não gerar desconfiança quando regras mudarem.
- Até onde o vocabulário fechado de tipos aguenta antes de precisar de subtipos.

## Próximos passos gerais e cuidados ao mexer

Sem ordem rígida: alinhar o código existente que ainda atualiza no lugar para passar a gravar eventos; mover os cálculos que hoje leem tabelas intermediárias para ler derivados reconstruíveis; revisar as tarefas Celery quanto à idempotência; e escrever os testes que garantem que reconstruir a partir dos eventos dá o mesmo resultado que o fluxo normal. Esse último teste é o mais importante e deve rodar sempre.

Ao mexer em `progress_events`, vale lembrar algumas coisas simples. Não adicione campo que misture fato com interpretação. Não crie caminho de edição "só para esse caso". Não leia eventos direto no painel por conveniência. Não use o índice de busca como fonte. Se algo disso parecer necessário, volte a esta nota e discuta antes, porque provavelmente é sinal de que falta um derivado ou um tipo de evento, e não de que a regra está errada.

Se a decisão precisar ser revista, que seja por motivo concreto, como custo de armazenamento que se mostre insustentável ou dificuldade real de operação, e não por desconforto com a mudança. Quando isso acontecer, registrar o que mudou em vez de apagar o que está aqui.
