---
id: 01KP6KB0GP7APS49JC5DJ53EM9
created: 2026-04-14T15:16-03:00
---

# standards-ingest: cuidados gerais ao mexer no componente

Anotação rápida do que costuma dar problema quando alguém altera o `standards-ingest`. Não é lista de decisões nem de requisitos, só os lugares onde a gente já tropeçou ou onde é fácil tropeçar. Se você vai mexer aqui, leia tudo antes, porque quase todos os pontos se tocam: o que parece mudança local no parser acaba aparecendo na busca, nas sugestões de exercícios e na tela do professor.

O `standards-ingest` pega documentos de currículo, de fontes diferentes e com formatos diferentes, e transforma em padrões estruturados que o resto do ClassroomCompass usa para medir o progresso do aluno. Ele fica no meio de vários sistemas: Django guarda o estado de verdade, Celery roda o trabalho pesado fora da requisição, Elasticsearch indexa para busca e o código com scikit-learn consome os padrões para sugerir o próximo exercício. Quem altera o `standards-ingest` sem lembrar dessa cadeia quebra algo longe do ponto onde mexeu.

## Dados de entrada e formato das fontes

A primeira coisa: as fontes não são nossas. Secretarias, redes de ensino e editoras publicam currículo do jeito que querem, e mudam o jeito sem avisar. Qualquer suposição sobre estrutura (ordem das colunas, nome dos campos, hierarquia de áreas, níveis e habilidades) vai quebrar em algum momento. Escreva o parser de forma que uma mudança de formato falhe alto e cedo, não que gere lixo silencioso.

Cuidado com codificação de texto. Currículo em português tem acento, cedilha, til, aspas tipográficas, travessões e espaços estranhos que vêm de copiar e colar de PDF ou de editor de texto. Normalização de Unicode importa: o mesmo texto visualmente igual pode chegar em formas diferentes e virar dois padrões distintos, ou pior, dois que não casam na comparação. Normalize na entrada, em um lugar só, e não espalhe tratamento pelo código.

Não confie em código de padrão como se fosse sempre único. Em algumas fontes o código se repete entre etapas ou entre componentes curriculares, e em outras o mesmo conteúdo tem código diferente conforme a edição do documento. Chave natural de verdade costuma ser uma combinação, e é fácil achar que o código sozinho basta até aparecer a colisão em produção.

Trate campos vazios, ausentes e com só espaços como casos diferentes. Já vimos descrição em branco virar padrão sem texto que depois aparece vazio na interface do professor. Decida em um ponto qual é o comportamento (rejeitar, marcar como incompleto, aceitar com aviso) e mantenha o mesmo em todos os caminhos de entrada.

Arquivos grandes e arquivos malformados: não carregue tudo em memória sem pensar, e não deixe um registro ruim derrubar o lote inteiro sem dizer qual era. Por outro lado, também não engula o erro e siga em frente como se nada tivesse acontecido. O meio-termo que funcionou melhor é registrar o registro problemático com contexto suficiente para alguém localizar na fonte e continuar com o resto, deixando claro no resultado final que houve rejeições.

Se a fonte vem de upload manual de alguém da equipe pedagógica, lembre que essa pessoa não vai ler log. A mensagem de retorno precisa ser compreensível para quem não é de tecnologia, em português claro, dizendo o que deu errado e o que fazer.

## Idempotência, versionamento e o que já está em uso

Esse é o ponto que mais machuca. Reexecutar uma ingestão com a mesma fonte tem que dar o mesmo resultado, sem duplicar padrões e sem apagar o que professores já usam. Antes de alterar qualquer coisa na lógica de gravação, pergunte: se isso rodar duas vezes seguidas, ou rodar parcialmente e ser repetido, o que acontece? Tarefas Celery podem ser reentregues, podem estourar tempo limite e ser reiniciadas, e podem rodar em paralelo se alguém disparar duas vezes. Escreva para esse mundo.

Padrões já referenciados por progresso de aluno não podem simplesmente sumir nem mudar de identidade. Se uma nova edição do currículo renomeia, divide ou funde padrões, o histórico dos alunos precisa continuar apontando para algo coerente. Remoção física é quase sempre errada aqui. Prefira marcar como substituído ou desativado e manter a relação com o que veio depois. Se você mudar a forma como identifica um padrão, pense no que acontece com tudo que foi gravado antes da mudança.

Versionamento de currículo: mais de uma edição da mesma norma pode coexistir, porque escolas diferentes migram em momentos diferentes. Não assuma que existe uma única versão vigente global. Qualquer consulta que escolhe a edição mais recente por conveniência pode dar resposta errada para uma escola que ainda não migrou.

Migrações de banco do Django que tocam tabelas de padrões merecem cuidado extra. Tabelas grandes com bloqueio longo durante a migração atrapalham o resto do sistema, e migração de dados que mexe em identificadores precisa ser reversível ou pelo menos testada com cópia de dados reais, anonimizados. Não misture mudança de esquema e transformação pesada de dados na mesma migração se der para separar.

Transações: gravar o padrão no banco e depois indexar no Elasticsearch são duas operações em sistemas diferentes, sem transação comum. Se a segunda falhar, ficam banco e índice divergentes. Dispare a indexação só depois do commit, e tenha um jeito de reconciliar depois. Já vimos tarefa que rodava antes do commit terminar e não encontrava o registro, ou encontrava uma versão antiga.

Também vale pensar em ingestão parcial. Se o lote tem muitas partes e uma falha no meio, o estado intermediário é visível para os professores? Em geral não deveria ser. Uma carga que deixa metade do currículo novo e metade do antigo visível produz sugestões de exercício incoerentes, e o professor não tem como saber por quê.

O mesmo vale para ordem. Padrões têm relações entre si (pré-requisitos, agrupamentos, hierarquia). Se a gravação não respeita a ordem em que as referências existem, aparecem ponteiros soltos ou ciclos. Valide as relações ao final da carga, não só registro a registro.

## Índice de busca, sugestões e quem consome os padrões

O Elasticsearch aqui é derivado, não fonte da verdade. Trate o índice como algo que dá para reconstruir a partir do banco. Mudança no mapeamento de campos, nos analisadores de texto ou na forma de tokenizar português quase sempre exige reindexar tudo, e isso precisa ser planejado para não deixar a busca fora do ar nem retornando resultados de dois mapeamentos misturados. Antes de mudar o mapeamento, pense em como fazer a troca sem janela de indisponibilidade perceptível para o professor.

Analisadores de português têm armadilhas próprias: radicalização agressiva junta palavras que o professor considera diferentes, e a falta dela faz a busca por uma habilidade não achar a mesma habilidade escrita no plural ou em outra flexão. Acentuação também: busca que ignora acento ajuda quem digita rápido, mas pode colapsar termos que são distintos. Qualquer ajuste nisso deve ser conferido com exemplos reais de termos de currículo, e não só com frases de teste inventadas.

O componente de sugestão de exercícios, que usa scikit-learn, depende da forma e do texto dos padrões. Se você muda a limpeza do texto, a segmentação ou a forma de associar padrões a exercícios, os vetores e os modelos treinados antes ficam desalinhados com o que passa a entrar. Isso não dá erro: as sugestões só pioram sem ninguém perceber. Quando mexer em qualquer coisa que afete o texto final do padrão, avise quem cuida do modelo e planeje retreinar ou, no mínimo, comparar o comportamento antes e depois com um conjunto de casos conhecidos.

Pense também no inverso: não otimize o texto do padrão para a busca e esqueça que ele é exibido ao professor. A mesma descrição aparece em tela, em relatório e em exportação. Truncar, forçar minúsculas ou remover pontuação para ajudar o índice não pode vazar para o que é mostrado. Mantenha o texto original e o texto normalizado como coisas separadas.

O front-end em Vue.js assume certas formas de resposta. Mudar nomes de campos, a profundidade da hierarquia ou a ordem em que os itens vêm pode quebrar telas de árvore de padrões, filtros e seleção em lote. Se for preciso mudar o contrato, faça de modo compatível por um tempo e remova o antigo depois, em vez de trocar tudo de uma vez junto com a ingestão.

Relatórios e painéis de progresso agregam por padrão, por área e por nível. Se a hierarquia muda, os agregados históricos mudam de significado. Conversar com quem usa os relatórios antes de reorganizar a árvore evita surpresa quando um professor compara o semestre atual com o anterior e os números não batem.

## Tarefas assíncronas, operação e testes

O trabalho do `standards-ingest` roda em Celery, então valem as regras de sempre para tarefas, mas com o agravante de que cargas de currículo são longas e raras. Isso dá uma falsa sensação de segurança: como quase ninguém executa, bug fica meses escondido. Cada tarefa deve ser segura para repetir, ter limite de tempo razoável e deixar rastro claro de início, progresso e fim. Argumentos de tarefa devem ser pequenos e serializáveis, referências a registros em vez de conteúdo grande, e a tarefa deve reler o estado do banco ao começar, não confiar no que estava no momento do disparo.

Filas: não coloque ingestão longa na mesma fila de tarefas curtas que o professor espera ver acontecer na hora. Uma carga de currículo que ocupa os trabalhadores atrasa coisas que o usuário percebe. Se mudar a forma como a ingestão se divide em subtarefas, observe o efeito na fila e na memória dos trabalhadores, porque dividir demais gera sobrecarga e dividir de menos gera tarefa que nunca termina.

Nova tentativa automática: configure com cuidado. Repetir erro de dados inválidos não adianta e só enche o log, enquanto falha temporária de rede ou do Elasticsearch merece nova tentativa com espera crescente. Diferencie os dois tipos de falha no código, em vez de capturar tudo igual.

Configuração e segredos de fontes externas ficam fora do código. Se a ingestão busca documentos de algum endereço remoto, trate o conteúdo baixado como não confiável: valide formato e tamanho antes de processar, não execute nada que venha dele e não confie em nomes de arquivo ou metadados vindos da fonte.

Testes: os melhores casos vêm de documentos reais que já deram problema, reduzidos ao mínimo e guardados como fixtures. Teste só com exemplos limpos dá confiança falsa. Inclua casos de acentuação estranha, campos vazios, duplicatas, renomeações entre edições e reexecução da mesma carga. Teste de idempotência merece atenção própria: rodar duas vezes e comparar o estado final. Teste de integração com Elasticsearch de verdade, e não só com dublê, pega problemas de mapeamento que o dublê nunca mostra.

Antes de liberar mudança relevante, rode a ingestão em um ambiente de homologação com dados parecidos com os reais e compare o resultado com o anterior: quantos padrões entraram, quais mudaram, quais sumiram, quais ficaram sem relação. Diferença grande sem explicação é sinal de que algo foi interpretado errado. Guarde essa comparação junto da mudança, para quem vier depois entender o que era esperado.

Por fim, observabilidade. Quando a ingestão falha, quem investiga geralmente não é quem escreveu. Logs com identificação da fonte, da edição e do registro, métricas de rejeições e de duração, e alerta para carga que não terminou valem mais do que parecem. Se você adicionar um novo caminho de entrada ou uma nova etapa, adicione junto o rastro dela, senão a próxima pessoa vai depurar no escuro.

Resumindo o que mais importa na pressa: não duplique, não apague o que está em uso, não assuma formato da fonte, não esqueça que busca e sugestões dependem do texto, e sempre pense no que acontece se a mesma coisa rodar de novo.
