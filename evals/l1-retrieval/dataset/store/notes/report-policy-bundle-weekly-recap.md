---
id: 01M3HJVGTNFQ6RH42K3W074X2P
created: 2026-09-27T11:03-03:00
---

# Resumo semanal: policy-bundle-repo

Resumo da semana no policy-bundle-repo, escrito rápido, no fim do expediente, para quem pegar o trabalho depois. Não tem número nem data de propósito. O que importa é o estado das coisas e o que ficou pendente. Quem precisar de detalhe fino vai olhar o histórico do repositório e os tickets, não aqui.

A semana foi mais de arrumação do que de coisa nova. O repositório continua sendo o lugar onde moram as políticas em Rego que o Open Policy Agent avalia contra as configurações de nuvem que o AuditMesh coleta, e o pacote que sai daqui é o que as funções Lambda baixam e carregam. Boa parte do que mexi foi para reduzir surpresa na hora de publicar o pacote, porque é ali que dói quando algo escapa. Também gastei tempo com testes de política que estavam frágeis, com a documentação das regras e com a conversa com o time que cuida da geração de tickets no Jira, que depende do formato das mensagens de violação que as nossas regras devolvem.

O texto abaixo está dividido por assunto e não por ordem cronológica. Onde eu não tinha certeza, escrevi que não tinha. Onde algo ficou pela metade, está dito como pela metade.

## Onde o policy-bundle-repo estava no começo da semana

Comecei a semana com o repositório num estado aceitável, mas com alguns incômodos que vinham de antes. O principal era que a estrutura de diretórios tinha crescido sem muito critério. Regras de armazenamento, de rede, de identidade e de criptografia estavam misturadas em alguns pontos, e algumas regras auxiliares eram importadas de lugares que não faziam sentido para quem lia pela primeira vez. Ninguém estava quebrado por isso, mas cada mudança pequena exigia abrir arquivos demais para entender o efeito.

O segundo incômodo era a cobertura de testes. Tínhamos testes de política que passavam, mas muitos deles verificavam só o caminho feliz: entrada com um recurso claramente violador, saída com uma violação. Faltava o oposto, o recurso claramente conforme que não deveria gerar nada, e faltavam os casos esquisitos, tipo atributo ausente, campo nulo, lista vazia, recurso que a coleta devolve com forma ligeiramente diferente conforme a região ou a forma como foi criado. Esses casos esquisitos são justamente os que geram ticket falso no Jira, e ticket falso é o que faz o time de segurança parar de confiar na ferramenta.

O terceiro ponto era o processo de publicação do pacote. Ele funciona, mas depende de uma sequência de passos que eu ainda não considero suficientemente à prova de erro. Há uma etapa de construção do pacote, uma de verificação e uma de envio para o lugar de onde as funções Lambda leem. Se a verificação for pulada por pressa, o erro só aparece em produção, quando a função tenta carregar o pacote e falha ou, pior, carrega e avalia errado sem reclamar. Foi aí que concentrei parte do esforço.

Por fim, havia a questão antiga de como as políticas devolvem as mensagens. Cada regra devolve um conjunto de campos que o código Python do lado do AuditMesh transforma em ticket. Algumas regras devolvem tudo o que o código espera; outras devolvem menos e o código Python compensa com valores padrão. Essa inconsistência é o tipo de coisa que só incomoda até o dia em que alguém muda o código Python sem saber que a regra dependia do padrão.

## Reorganização e limpeza das regras

A reorganização foi feita aos poucos, sem mover tudo de uma vez. Decidi não fazer uma mudança grande num único conjunto de alterações, porque a revisão fica impossível e, se algo der errado, é difícil saber o que causou. Em vez disso, fui por agrupamentos pequenos: primeiro as regras auxiliares, depois as regras de um domínio de cada vez.

As regras auxiliares eram o ponto mais sujo. Havia funções que faziam quase a mesma coisa com nomes diferentes, escritas em momentos diferentes por pessoas diferentes. Juntei as que eram claramente equivalentes e deixei as que tinham diferença sutil de comportamento, anotando a diferença em comentário para que a próxima pessoa não tente unificar sem pensar. Uma delas, que checa se um recurso tem determinada etiqueta, tratava de forma diferente o caso de a etiqueta existir com valor vazio. Não mexi no comportamento; só deixei explícito. Se alguém decidir que o comportamento deve mudar, que seja uma decisão consciente e com teste.

Nos domínios, comecei por armazenamento porque é o que tem mais regras e mais histórico de falso positivo. Separei as regras por tipo de preocupação: acesso público, criptografia, versionamento e retenção, registro de acesso. Cada grupo agora tem seu próprio arquivo e seu próprio arquivo de teste ao lado, com nomes que dizem o que cobrem. Foi uma mudança mecânica na maior parte, mas encontrei algumas regras duplicadas, onde duas regras diferentes acabavam gerando violação para o mesmo recurso pelo mesmo motivo. Isso faria dois tickets para o mesmo problema. Mantive a mais clara e removi a outra, depois de conferir que os testes existentes continuavam passando e de adicionar um teste que prova que o recurso gera uma única violação.

Rede e identidade ficaram para a próxima rodada. Olhei por cima e há bastante coisa para arrumar, principalmente em identidade, onde algumas regras fazem comparação de texto em políticas de permissão de um jeito que parece funcionar por acaso. Vou precisar de tempo para entender direito antes de mexer. Deixei anotado no próprio repositório, em comentário curto perto das regras, o que me pareceu estranho, para não depender da minha memória.

Um cuidado que tomei durante toda a reorganização: não renomear pacotes de política que são referenciados de fora. Se o código Python ou alguma configuração do lado das funções Lambda consulta uma regra pelo nome do pacote, renomear quebra silenciosamente, porque o OPA simplesmente devolve resultado indefinido e o código interpreta como ausência de violação. Isso seria o pior tipo de falha, a que parece sucesso. Por isso, onde havia dúvida, mantive o nome externo e só mexi por dentro. Ficou uma pendência: levantar com calma a lista do que é consultado de fora, para poder renomear com segurança no futuro. Hoje essa lista só existe na cabeça de quem já trabalhou nisso há mais tempo.

Outra coisa que apareceu na limpeza: comentários desatualizados. Havia regras cujo comentário descrevia um comportamento que a regra não tinha mais. Corrigi os que consegui confirmar e apaguei os que eram só ruído. Comentário errado é pior que nenhum, então apaguei sem dó quando não tinha como verificar.

## Testes de política

O trabalho nos testes foi o que mais rendeu. A ideia foi simples: para cada regra tocada na semana, garantir pelo menos um caso de violação, um caso conforme e um caso de borda. Não consegui fazer isso para todas as regras do repositório, e não era o objetivo. Fiz para as regras de armazenamento, que estavam na reorganização, e para algumas regras de criptografia que tinham histórico de reclamação.

Os casos de borda mais produtivos vieram de olhar entradas reais anonimizadas que o time de coleta compartilhou. Percebi que a forma dos dados varia mais do que os testes antigos assumiam. Alguns recursos chegam sem determinado bloco de configuração em vez de chegarem com o bloco vazio; outros chegam com o bloco presente, mas com valores que significam padrão da plataforma. As regras antigas tratavam esses casos de maneiras inconsistentes: umas consideravam ausência como violação, outras como conformidade. Em alguns casos a escolha fazia sentido, em outros era acidente. Documentei a escolha caso a caso em comentário de teste e, quando a escolha parecia errada, abri conversa em vez de mudar sozinho, porque mudar o que conta como violação muda o volume de tickets que as pessoas recebem.

Também melhorei a legibilidade dos testes. Muitos tinham entradas enormes copiadas de coleta real, com campos que nada tinham a ver com a regra testada. Reduzi para o mínimo necessário para exercitar a regra. O ganho é duplo: o teste fica mais fácil de ler e fica claro o que a regra realmente lê. Aproveitei para comentar, em frases curtas, por que cada entrada existe. Quem abrir um teste que falhou daqui a alguns meses precisa entender rápido se o teste está certo e a regra errada ou o contrário.

Tive um episódio de teste instável. Um teste passava e falhava dependendo da ordem em que o conjunto era executado, o que no mundo de Rego costuma significar que alguma regra depende de dado global mal isolado, ou que um teste está sobrescrevendo uma entrada compartilhada. Achei a causa: um teste usava um dado de apoio que outro teste também alterava por simulação. Isolei os dois. Não é bonito, mas está resolvido, e deixei a observação para a próxima pessoa não cair na mesma.

Sobre desempenho, fiz só uma passada de olho. Há regras que iteram sobre coleções grandes de recursos de um jeito que pode ficar lento quando a conta tem muita coisa. Como as funções Lambda têm tempo limitado de execução, isso importa de verdade. Não medi com seriedade nesta semana, então não vou afirmar nada; só marquei as regras suspeitas para uma medição futura com dados representativos. Se o time de plataforma tiver um conjunto de dados grande e anonimizado, vale usar.

O que ficou faltando nos testes: regras de rede e identidade, como já disse, e um teste de integração que carregue o pacote inteiro como as funções Lambda carregam e rode uma amostra de entradas de ponta a ponta. Hoje os testes rodam por regra, o que é ótimo para achar o erro, mas não pega problemas de pacote como um arquivo que não entrou na construção ou uma dependência entre pacotes que só existe no ambiente de desenvolvimento.

## Construção e publicação do pacote

Na publicação, o objetivo da semana foi diminuir a chance de empurrar um pacote quebrado. Não mudei a arquitetura, só reforcei os pontos fracos.

Primeiro, a verificação sintática e de tipos das políticas passou a ser obrigatória antes da construção. Antes ela era um passo que a gente rodava por hábito, e quem esquecia só descobria depois. Agora a construção falha se a verificação falhar. Parece óbvio e deveria ter sido assim desde o começo.

Segundo, acrescentei uma checagem que compara o conjunto de regras que estão no pacote construído com o conjunto esperado a partir do repositório. A motivação foi um susto antigo: um arquivo ficou de fora da construção por causa de um padrão de exclusão amplo demais, e a regra simplesmente não existia em produção. Nenhum erro, nenhuma violação encontrada, ninguém percebeu por um tempo. A checagem nova não é sofisticada; só confere que nada esperado sumiu. Mas já pegou uma coisa durante o teste, o que me deu confiança de que vale o custo.

Terceiro, conversei sobre a forma de versionar o pacote publicado. Hoje existe uma convenção, mas ela depende de a pessoa lembrar de seguir. Meu desejo é que a identificação do pacote seja derivada do conteúdo ou do estado do repositório, e não digitada. Isso permite, do lado das funções Lambda, registrar exatamente qual pacote avaliou cada varredura, o que ajuda muito quando alguém pergunta por que um recurso gerou ou deixou de gerar ticket em determinado momento. Ainda é só proposta. Não implementei, e não quero registrar aqui como decisão tomada, porque o time ainda não opinou.

Quarto, revisei como as funções Lambda lidam com um pacote inválido ou ausente. O comportamento atual é razoável: se não conseguem carregar a versão nova, continuam com a anterior e registram o problema. Mas o registro desse problema ficava misturado com outros avisos e era fácil de passar batido. Sugeri ao time responsável um alerta separado. Eles acharam bom e vão ver como encaixar. Do nosso lado, só garanti que o pacote deixe claro, nos metadados, o que ele é, para a mensagem de log fazer sentido.

Uma coisa que continua me incomodando: o processo de reverter. Se um pacote ruim chegar a produção, voltar para o anterior depende de alguém saber onde está o anterior e como apontar as funções para ele. Existe, mas não está escrito de forma que alguém de plantão leia e execute sem pensar. Comecei um rascunho de roteiro de reversão em linguagem simples, sem comandos específicos nesta nota, e ele ainda precisa de revisão de quem opera as funções.

## Contrato com o Jira e com o código Python

O lado das mensagens de violação foi a conversa mais delicada da semana, porque envolve outras pessoas. As regras devolvem estruturas com informações sobre o recurso afetado, o motivo, a severidade e uma sugestão de correção. O código Python lê isso e monta o ticket no Jira. Qualquer inconsistência vira ticket com campo vazio, título estranho ou, pior, ticket duplicado.

O que fiz foi levantar, regra por regra nas áreas que mexi, quais campos eram devolvidos e quais eram esperados. Encontrei regras que devolviam a sugestão de correção em texto livre longo, outras em frase curta e algumas sem sugestão nenhuma. Para quem recebe o ticket, a diferença é grande: uma sugestão boa economiza uma investigação inteira. Propus um padrão de redação, curto e direto, dizendo o que mudar, sem jargão desnecessário. Apliquei nas regras de armazenamento e deixei as outras para depois, para não misturar reorganização com mudança de texto visível ao usuário final.

Sobre severidade, há um desacordo ainda aberto. Algumas regras definem a severidade dentro da própria política; em outras, o código Python calcula a severidade a partir de outras informações. Ter as duas coisas é confuso, porque não fica claro quem manda. Minha inclinação é que a política seja a fonte, já que quem escreve a regra sabe melhor a gravidade, mas há argumento do outro lado: o código Python consegue considerar contexto da conta, como ambiente de produção ou de teste, que a regra isolada não vê. Não resolvi, e não vou fingir que resolvi. Anotei os dois argumentos e levei para a conversa com o time.

Também falei sobre deduplicação. O AuditMesh evita abrir ticket repetido guardando no DynamoDB uma chave que identifica a violação. Essa chave é montada a partir de campos que a regra devolve. Se eu mudar o que a regra devolve, posso mudar a chave sem querer, e aí todas as violações antigas parecem novas e o Jira recebe uma enxurrada de tickets duplicados. Isso é um risco concreto de qualquer mudança em mensagens. Por isso, nesta semana, qualquer alteração que fiz no texto ou na estrutura foi conferida contra o que alimenta a chave. Onde havia dúvida, não alterei o campo que entra na chave e só alterei campos de apresentação. Vale transformar isso em regra do repositório: documentar quais campos são parte da identidade da violação e proteger com teste que falhe se mudarem.

O time do Jira também pediu, informalmente, que as mensagens trouxessem um pouco mais de contexto sobre o responsável provável pelo recurso, quando a etiqueta permite inferir. Achei razoável, mas isso exige que a regra leia etiquetas que hoje ignora, e a coleta nem sempre traz essas etiquetas. Ficou como ideia a avaliar depois de entender o que a coleta entrega de forma confiável.

## Problemas encontrados e dúvidas em aberto

Algumas coisas apareceram durante a semana e não foram resolvidas. Listo sem ordem de importância.

Há uma regra de criptografia que gera violação para recursos que, na prática, são criados por um serviço gerenciado e não podem ser alterados pelo cliente. O time de segurança reclamou algumas vezes de tickets que ninguém consegue resolver. A saída óbvia seria excluir esses recursos, mas excluir por critério frágil, como padrão de nome, é perigoso: alguém cria um recurso com nome parecido e escapa da verificação. Quero uma exclusão baseada em atributo confiável do recurso, e ainda não sei se a coleta traz esse atributo. Precisa de conversa com quem cuida da coleta.

Há também a questão das exceções aprovadas. Equipes às vezes têm motivo legítimo para manter uma configuração que viola a política, e hoje o jeito de registrar isso é espalhado: parte no Jira, marcando o ticket como aceito, parte em listas dentro do repositório. Ter listas de exceção dentro do policy-bundle-repo mistura dado que muda com frequência e por motivo de negócio com código de política que deveria mudar com cuidado. Minha preferência é que as exceções venham como dado externo, carregado junto com a avaliação, com dono e motivo registrados. Mas isso é uma mudança de desenho que não cabe resolver numa semana, e eu não quero atropelar. Só deixo registrado que o problema existe e cresce.

Outro ponto é o tratamento de recursos em regiões ou contas que a coleta não conseguiu ler. Hoje, se a coleta falha, a entrada simplesmente não existe, e a política não tem como dizer que não conseguiu verificar. O resultado é um falso sentimento de conformidade. Isso é mais um problema do desenho do AuditMesh como um todo do que do repositório de políticas, mas mexe com o que as regras devolvem, então mencionei. Uma possibilidade é a coleta marcar explicitamente o que ficou sem leitura, e a política produzir um tipo de resultado diferente, de verificação incompleta. Por enquanto é só conversa.

Sobre dependências, notei que o conjunto de ferramentas que usamos para testar e construir políticas está ficando defasado em relação ao que o projeto do OPA vem publicando. Não é urgente e nada quebrou, mas atualizar sempre traz o risco de mudança sutil de comportamento na avaliação. Se for atualizar, quero fazer isolado de qualquer mudança de regra, com os testes todos rodando antes e depois, para saber que uma diferença vem da ferramenta e não de edição minha. Deixei como item para uma semana mais calma.

Uma dúvida menor: a convenção de nomes das regras. Hoje há mistura de estilos, e a reorganização tornou isso mais visível. Não quero bikeshedding, mas um guia curto de nomes ajudaria quem entra no projeto. Pode ser um parágrafo no documento de contribuição.

Por fim, uma observação sobre revisão. Várias mudanças desta semana são grandes em linhas mas pequenas em efeito, porque movem coisa de lugar. Para o revisor, isso é cansativo e perigoso: no meio de movimento de código, uma mudança real de comportamento passa despercebida. Tentei separar sempre movimento de alteração, em conjuntos distintos, e pedi para quem revisa olhar os de alteração com mais atenção. Funcionou razoavelmente. Vale manter o hábito.

## Próximos passos

O que pretendo fazer a seguir, mais ou menos em ordem de prioridade, sem compromisso de prazo.

Primeiro, terminar a reorganização dos domínios que faltam, rede e identidade, com o mesmo cuidado de separar movimento de alteração e de não renomear nada que seja consultado de fora. Antes de começar, levantar de verdade a lista do que é referenciado externamente, perguntando a quem mantém o código Python e a configuração das funções Lambda. Se essa lista existir escrita, as próximas reorganizações ficam muito mais seguras.

Segundo, o teste de integração do pacote inteiro. Quero algo que carregue o pacote construído do mesmo jeito que as funções o carregam e avalie uma pequena amostra de entradas representativas, comparando com resultados esperados guardados no repositório. Não precisa ser grande. Precisa pegar o tipo de falha que os testes por regra não pegam, como arquivo faltando, dependência quebrada entre pacotes ou diferença entre o ambiente local e o de execução.

Terceiro, proteger a identidade das violações. Documentar quais campos compõem a chave de deduplicação e escrever um teste que falhe se alguém mudar esses campos numa regra existente. Isso reduz bastante o risco de uma enxurrada de tickets duplicados por uma edição inocente de mensagem.

Quarto, fechar a conversa sobre severidade com o time do código Python. Preciso de uma decisão, qualquer uma razoável, desde que seja uma só. Enquanto isso não acontece, continuo sem mexer na severidade das regras existentes, para não criar mais divergência.

Quinto, terminar e revisar o roteiro de reversão do pacote com quem opera as funções, e ver se o alerta de falha de carga entra do lado deles. Esse item é o que mais reduz risco para o custo de esforço, e por isso quero que não fique esquecido atrás de coisas mais interessantes.

Sexto, olhar o desenho das exceções aprovadas. Não para implementar já, mas para escrever uma proposta curta com as opções e os riscos de cada uma, e levar para discussão. Se a decisão for tirar as listas de dentro do repositório, isso muda bastante a forma como as regras são escritas, então melhor decidir antes de continuar polindo as regras atuais.

Sétimo, quando houver tempo e dados, medir o desempenho das regras marcadas como suspeitas. Sem medição, qualquer otimização é chute, e já vi otimização deixar a regra mais difícil de ler sem ganho real.

Por último, a atualização das ferramentas, isolada, em uma semana tranquila.

## Notas para quem pegar o trabalho

Algumas observações práticas, para economizar tempo de quem continuar.

Antes de mexer em qualquer regra, leia o teste dela. Se não houver teste, escreva um antes de alterar, nem que seja um só, para fixar o comportamento atual. Várias das surpresas desta semana vieram de regras cujo comportamento real era diferente do que o nome sugeria.

Desconfie de ausência. Em Rego, quando uma condição não se satisfaz, o resultado é indefinido, não falso, e isso se propaga de maneiras que enganam. Uma regra que parece dizer que algo é conforme pode estar apenas deixando de dizer qualquer coisa. Os casos de borda dos testes existem para isso, e vale o esforço de escrevê-los com cuidado.

Não confie em que a coleta sempre entrega os dados na mesma forma. Ela mudou ao longo do tempo e pode mudar de novo, e nem sempre avisa. Quando uma regra depende de um campo, escreva em comentário de onde vem esse campo e o que acontece se faltar.

Quando mudar texto de mensagem, pergunte a si mesmo se aquele campo entra na identidade da violação. Se não souber, pergunte antes. Um ticket duplicado em massa é difícil de desfazer, porque as pessoas já receberam notificação e já começaram a reagir.

Mantenha as mudanças pequenas e separe movimento de alteração. A revisão agradece, e o diagnóstico depois também, quando alguém precisar descobrir qual mudança trouxe um comportamento novo.

E registre dúvida como dúvida. Nesta semana tive vontade, algumas vezes, de resolver rápido uma questão que na verdade dependia de decisão de outras pessoas, como severidade, exceções e exclusão de recursos gerenciados. Resolver sozinho teria economizado conversa e custado retrabalho depois. Prefiro deixar a pendência escrita, com os argumentos, a tomar uma decisão por omissão dentro do código de política, onde ela fica escondida e vira regra de fato sem que ninguém tenha aprovado.

O estado final da semana: o policy-bundle-repo está mais organizado nas regras de armazenamento, com testes melhores e uma publicação um pouco mais difícil de errar. Rede e identidade continuam como estavam, e as questões de desenho que envolvem outras equipes seguem abertas. Nada do que foi feito mudou a quantidade ou o tipo de ticket de forma intencional, e se alguém notar mudança de volume no Jira nos próximos dias, vale me avisar logo, porque pode ser uma das duplicidades que removi ou um efeito que eu não previ.
