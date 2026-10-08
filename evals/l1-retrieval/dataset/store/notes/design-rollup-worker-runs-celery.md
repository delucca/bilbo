---
id: 01KFF7X2CK99XQKQ7E7NZA356V
created: 2026-01-20T23:59-03:00
---

# Design do rollup-worker

O rollup-worker é o processo do ClassroomCompass que consolida, por turma, os eventos de progresso dos alunos em relação aos padrões curriculares. Ele roda pelo agendador beat do Celery todas as noites às `02:30` e agrega os eventos de progresso por turma. Esta nota registra como ele foi pensado, por que foi pensado assim e quais armadilhas já são conhecidas. Foi escrita com pressa, então é direta e não tenta ser um manual completo.

A ideia central: o professor abre o painel de manhã e precisa ver o retrato da turma sem esperar nenhuma conta pesada. Quem faz a conta pesada é o rollup-worker, de madrugada, quando ninguém está usando o sistema. Durante o dia o painel só lê o resultado já pronto.

O que o rollup-worker não faz: ele não recebe eventos, não valida eventos e não decide qual exercício sugerir. Receber e validar é trabalho da API em Django. Sugerir é trabalho do componente de recomendação. O rollup-worker fica no meio, só agregando.

## Contexto e objetivo

O ClassroomCompass acompanha o progresso de estudantes do ensino médio contra padrões curriculares e sugere os próximos exercícios. Cada vez que um aluno resolve um exercício, entrega uma atividade ou é avaliado pelo professor, o backend em Django grava um evento de progresso. Esses eventos são pequenos, numerosos e chegam o dia inteiro. Olhar evento por evento não serve para o professor: ele quer saber como a turma está em cada padrão, quem está atrasado, quem já domina o conteúdo e onde a turma inteira travou.

O rollup-worker existe para transformar o fluxo bruto de eventos em números por turma. O resultado alimenta três consumidores:

- o painel em Vue.js que o professor usa para ver a turma;
- o motor de sugestão de exercícios, que usa modelos do scikit-learn e precisa de entradas já consolidadas;
- as buscas e filtros no Elasticsearch, que mostram turmas e padrões ordenados por situação.

O objetivo de projeto é simples de dizer: depois que o rollup-worker termina a rodada da noite, todo número que o professor vê pela manhã vem de uma única fotografia consistente dos eventos, e não de leituras feitas em momentos diferentes.

## Agendamento

O rollup-worker é disparado pelo agendador beat do Celery, todas as noites às `02:30`. Esse horário foi escolhido porque cai bem depois do fim do uso intenso das escolas e bem antes de os professores começarem o dia. Também fica longe de outras rotinas noturnas de manutenção, para não disputar banco de dados e disco com elas.

Pontos importantes sobre o agendamento:

- O beat só dispara a tarefa. Quem executa é um worker comum do Celery. Se não houver worker disponível na hora, a tarefa espera na fila e começa quando houver, então um atraso de infraestrutura vira atraso do rollup, mas não vira perda.
- A tarefa não deve depender de que o disparo aconteça exatamente no minuto marcado. O código toma como referência o intervalo de eventos a processar, e não o relógio do momento em que começou. Assim, se rodar atrasado, o resultado é o mesmo.
- Só pode haver um beat ativo. Dois beats ao mesmo tempo disparariam a rodada em duplicidade. A proteção contra isso é dupla: a operação garante uma única instância do beat, e a própria tarefa usa um bloqueio para recusar execução concorrente da mesma rodada.
- O fuso horário do agendador precisa ser o mesmo configurado para o projeto Django. Vale conferir sempre que mexer em configuração de Celery ou de deploy.

Se a rodada da noite falhar, a política é não tentar compensar no meio do dia automaticamente. O professor continua vendo os números da rodada anterior, claramente marcados com a data em que foram calculados, e a próxima rodada da noite recupera tudo, porque ela sempre processa desde onde a anterior parou.

## Fluxo da agregação

A rodada segue uma sequência fixa. A ordem importa porque cada etapa assume que a anterior terminou.

### Definição da janela

No começo, a tarefa decide qual conjunto de eventos entra na rodada. Ela guarda uma marca de progresso: até que ponto os eventos já foram consolidados. A janela vai dessa marca até um limite superior fixado no início da rodada. Eventos que chegam depois que o limite foi fixado ficam para a rodada seguinte, mesmo que entrem no banco enquanto a rodada ainda está rodando. Isso evita contar um evento pela metade, ou contá-lo duas vezes.

### Leitura por turma

Os eventos são lidos agrupados por turma. Turma é a unidade natural de trabalho: o professor pensa em turmas, as permissões são por turma e os dados de uma turma não dependem dos de outra. Por isso a leitura e a agregação são divididas em pedaços independentes, um por turma, que podem ser executados em paralelo pelos workers.

A leitura usa o banco relacional gerenciado pelo Django. Há cuidado para não carregar todos os eventos de uma turma grande na memória de uma vez: a leitura é feita em lotes, e as somas parciais são acumuladas aos poucos.

### Agregação

Para cada turma, o rollup-worker calcula, por aluno e por padrão curricular, um resumo do progresso: quantas tentativas, qual o resultado mais recente, qual a tendência recente e se o padrão pode ser considerado dominado segundo a regra pedagógica vigente. Depois sobe um nível e calcula o resumo da turma por padrão: proporção de alunos que dominam, proporção em andamento e proporção que ainda não começou.

As regras pedagógicas de domínio ficam em configuração e em código do domínio, não espalhadas pelo rollup-worker. Ele chama essas regras, não as reimplementa. Se a equipe pedagógica mudar o critério, a mudança é feita na regra, e a rodada seguinte já reflete o novo critério para os dados novos.

### Gravação do resultado

O resultado de cada turma é gravado de forma atômica: ou todos os números da turma naquela rodada entram, ou nenhum entra. Isso garante que o painel nunca mostre uma turma com metade dos padrões atualizados e metade antigos. A gravação é idempotente: rodar a mesma janela duas vezes produz o mesmo estado final, porque o resumo é recalculado e substituído, e não somado em cima do anterior.

### Publicação para busca

Depois de gravar no banco, o rollup-worker atualiza o índice no Elasticsearch com os resumos por turma e por padrão. A ordem é deliberada: o banco é a fonte de verdade e o índice é derivado. Se a atualização do índice falhar, o dado correto continua no banco e a próxima rodada reindexa. Nunca se grava primeiro no índice.

### Avanço da marca

Só no fim, depois que todas as turmas foram gravadas e indexadas, a marca de progresso avança. Se qualquer pedaço falhar, a marca não avança, e a próxima rodada reprocessa a janela inteira. Como a gravação é idempotente, reprocessar é seguro.

## Decisões e motivos

### Agregar à noite em vez de em tempo real

A alternativa era atualizar os números a cada evento. Foi descartada por três motivos. Primeiro, o volume de eventos durante o horário de aula é alto e concentrado, e o banco já está ocupado com a API. Segundo, o professor não precisa de precisão de segundos: um retrato fechado na madrugada é suficiente para planejar o dia. Terceiro, um cálculo em lote é mais fácil de testar, de repetir e de explicar. Quando alguém pergunta por que um número é aquele, dá para refazer a rodada e comparar.

O custo dessa escolha é conhecido: se um professor corrige uma nota no meio da manhã, o painel só reflete isso no dia seguinte. Para essa necessidade, a interface mostra o dado individual do aluno direto dos eventos, e deixa claro que o resumo da turma é do fechamento da noite anterior. Se um dia o produto exigir atualização durante o dia, o caminho mais provável é uma rodada incremental só da turma afetada, reaproveitando as mesmas funções de agregação, e não trocar o desenho inteiro.

### Dividir o trabalho por turma

Dividir por turma dá paralelismo simples e isolamento de falhas. Uma turma com dado estranho não derruba as outras. A desvantagem é que existem muitas tarefas pequenas, e o custo de enfileirar cada uma aparece. Para equilibrar, turmas pequenas podem ser agrupadas em um mesmo pedaço de trabalho, enquanto turmas grandes ficam sozinhas. O critério de agrupamento é volume de eventos, não escola.

### Idempotência acima de eficiência

A agregação recalcula o resumo em vez de somar incrementos sobre o resumo anterior. Isso gasta mais processamento, mas elimina uma classe inteira de erros: evento duplicado, rodada repetida, rodada interrompida no meio. Num sistema com fila e reenvio automático de tarefas, a repetição acontece, então o desenho assume que vai acontecer.

### Banco como fonte de verdade

O Elasticsearch é rápido para buscar e ordenar, mas não é onde se guarda o número oficial. Todo número exibido pode ser reconstruído a partir do banco. O índice pode ser apagado e refeito sem perda.

### Regras pedagógicas fora do worker

O worker é infraestrutura de agregação, e quem entende de currículo é o domínio. Manter as regras separadas evita que uma alteração pedagógica exija mexer em código de fila e agendamento, e permite testar a regra sem Celery.

## Relação com o resto do sistema

### Django

O rollup-worker usa os modelos do Django para ler eventos e gravar resumos, e compartilha as configurações do projeto. Mudanças de esquema nas tabelas de eventos ou de resumos precisam ser coordenadas com ele. Uma migração que renomeie campo lido pela agregação pode quebrar a rodada da noite sem que nenhum teste da API perceba. Por isso a agregação tem testes próprios que rodam sobre os modelos reais.

### Celery

Além do beat, o Celery fornece as filas e os workers. A rodada da noite não deve ficar na mesma fila das tarefas que o professor dispara de forma interativa, como gerar um relatório sob demanda. Se ficasse, uma rodada grande poderia atrasar uma ação que alguém está esperando na tela. Há fila separada para o rollup-worker, com workers dimensionados para a carga da madrugada.

### scikit-learn

O motor de sugestão usa modelos do scikit-learn que consomem os resumos produzidos pelo rollup-worker como entrada. O rollup-worker não treina nem aplica modelos. A dependência é de dados: se os resumos não forem atualizados, as sugestões do dia ficam baseadas no retrato anterior. Por isso a rodada da noite termina o rollup antes de qualquer rotina de recomendação que dependa dele, e essa ordem precisa ser mantida se novas rotinas forem agendadas.

### Elasticsearch

O índice recebe os resumos já calculados. O rollup-worker cuida de criar ou atualizar os documentos por turma e por padrão, e de remover documentos de turmas que deixaram de existir. A estrutura dos documentos é combinada com o painel: mudar nome de campo no índice exige mudar o Vue.js junto.

### Vue.js

O painel só lê. Ele mostra a data e a hora do último fechamento ao lado dos números da turma, para o professor saber de quando é a informação. Esse carimbo vem do rollup-worker, que o grava ao fim de cada rodada bem-sucedida. Se o carimbo estiver velho demais, o painel avisa discretamente que a atualização da noite não aconteceu.

## Falhas, observação e armadilhas

### Como a rodada falha

As falhas mais prováveis:

- indisponibilidade temporária do banco ou do Elasticsearch durante a madrugada, normalmente por manutenção;
- falta de worker na fila certa, por deploy mal feito ou por dimensionamento;
- evento com dado inconsistente, por exemplo ligado a um padrão curricular que foi retirado;
- turma muito grande que estoura o tempo limite de um pedaço de trabalho.

Para indisponibilidade temporária, os pedaços têm nova tentativa automática com espera crescente. Para dado inconsistente, o evento ruim é registrado e pulado, a turma segue sem ele e a ocorrência vira alerta para a equipe, em vez de bloquear a turma inteira. Para tempo limite, a solução é dividir melhor o trabalho, e não aumentar o limite sem critério.

### O que olhar de manhã

A verificação mais útil é comparar o carimbo do último fechamento com a data de hoje. Depois, conferir se o número de turmas processadas bate com o de turmas ativas. Uma diferença indica turma pulada ou falha parcial. Os registros da tarefa mostram, por turma, se ela terminou, quanto tempo levou e quantos eventos entraram.

### Armadilhas conhecidas

- **Fuso horário.** O horário `02:30` é interpretado no fuso configurado para o agendador. Mudar o fuso do projeto sem rever o agendamento desloca a rodada. Se houver horário de verão, vale conferir como a configuração trata o dia da mudança, para a rodada não rodar duas vezes nem ser pulada.
- **Eventos tardios.** Um evento que chega com data antiga, por exemplo de um aplicativo que ficou offline, deve entrar na janela pelo momento em que foi recebido, e não pela data em que aconteceu. Caso contrário ele cairia numa janela já fechada e nunca seria contado. O resumo, porém, é calculado com a data real do evento. Essa distinção entre recebimento e ocorrência é fácil de confundir ao mexer na consulta.
- **Turmas e alunos que mudam.** Aluno que troca de turma no meio do período tem eventos em duas turmas. A regra atual é atribuir cada evento à turma em que o aluno estava quando o evento aconteceu. Qualquer mudança nisso altera os números históricos e precisa de decisão pedagógica, não só técnica.
- **Padrões curriculares revisados.** Quando a lista de padrões muda, resumos antigos podem apontar para padrões que não existem mais. O rollup-worker não apaga o histórico sozinho; a limpeza é uma operação separada e deliberada.
- **Reprocessamento manual.** É possível mandar reprocessar uma janela ou uma turma. Como a agregação é idempotente, isso é seguro, mas consome recursos que a rodada normal também usa. Reprocessamento grande em horário de aula pode deixar a API lenta.
- **Fila compartilhada.** Se alguém mover a tarefa para a fila geral por conveniência, ela volta a competir com tarefas interativas. É um defeito de configuração fácil de reintroduzir sem perceber.
- **Índice fora de sincronia.** Se o painel mostrar número diferente do que o banco mostra, a primeira hipótese é índice atrasado por falha na etapa de publicação. A correção é reindexar a partir do banco, nunca editar o índice à mão.

## Pontos em aberto

Algumas decisões ficaram pendentes e merecem retorno quando houver tempo:

- Avaliar uma rodada incremental durante o dia, só para turmas com muita atividade, caso os professores peçam números mais frescos. Antes, medir quantas vezes o atraso de um dia realmente atrapalha o planejamento.
- Rever o critério de agrupamento de turmas pequenas, se o número de tarefas enfileiradas continuar alto em períodos de provas.
- Definir uma política de retenção para resumos antigos, para o banco e o índice não crescerem sem limite ao longo dos anos letivos.
- Melhorar o aviso ao professor quando a rodada falha: hoje o painel mostra o carimbo antigo, mas não explica o motivo nem diz quando deve voltar ao normal.
- Documentar, junto com a equipe pedagógica, a regra de domínio de um padrão, hoje conhecida mais pelo código do que por um texto de referência.

Quem for alterar o rollup-worker deve lembrar de três princípios que sustentam o desenho todo: a janela de eventos é fixada no começo da rodada, a gravação é atômica por turma e idempotente, e a marca de progresso só avança quando tudo terminou. Quebrar qualquer um deles traz de volta os problemas de contagem dupla, números pela metade e dados que somem sem aviso, que o desenho atual foi feito para evitar.
