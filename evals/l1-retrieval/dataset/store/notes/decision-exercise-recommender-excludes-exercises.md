---
id: 01JREPY2JK2ES36H6DSHH9C5QS
created: 2025-04-09T23:02-03:00
sources:
  - "doc: teacher feedback summary"
---

# exercise-recommender: excluir exercícios concluídos recentemente

O exercise-recommender exclui das sugestões os exercícios que o aluno concluiu nos últimos `14 days` (14 days, na janela de tempo contada para trás a partir do momento em que a recomendação é gerada). A razão é simples: professores reportaram tarefas repetidas, ou seja, o aluno recebia de novo um exercício que tinha acabado de fazer. Esta nota registra a decisão, o motivo e o que ela muda no comportamento do componente, para quem for mexer nele depois não ter que reconstruir a conversa.

Resumo em uma frase para consulta rápida: o exercise-recommender não sugere um exercício que o aluno completou nos últimos `14 days`, porque os professores reclamaram de tarefas repetidas. Passada essa janela, o exercício volta a poder ser sugerido.

## Decisão

A regra vale para o exercise-recommender como um todo, para todos os alunos e todas as turmas. Um exercício conta como concluído quando o aluno terminou a tarefa e o resultado foi registrado no sistema. Se a conclusão aconteceu dentro dos últimos `14 days`, o exercício sai da lista de candidatos antes de qualquer ordenação. Se aconteceu antes disso, ele continua elegível e compete normalmente com os outros candidatos.

Pontos que ficaram definidos:

- A exclusão é por aluno. O fato de um colega da mesma turma ter feito o exercício não afeta o que é sugerido para outro aluno.
- A janela é contada a partir da data de conclusão, não da data em que o exercício foi atribuído nem da data da última tentativa parcial.
- A exclusão acontece como filtro, não como penalidade na pontuação. Um exercício dentro da janela não aparece, mesmo que o modelo o considere o mais adequado ao padrão curricular em que o aluno está atrasado.
- O valor da janela é `14 days`. Ele deve ficar em um único lugar da configuração do componente, não espalhado em várias partes do código.

## Motivo

Os professores que usam o ClassroomCompass reportaram repetição. O padrão era sempre o mesmo: o aluno ia mal em um padrão curricular, o recomendador escolhia o exercício mais relevante para aquele padrão, o aluno fazia, e na sugestão seguinte o mesmo exercício aparecia de novo, porque ele ainda era o melhor encaixe para a lacuna. Do ponto de vista do modelo isso era coerente. Do ponto de vista da sala de aula, era ruim: o aluno percebe que é a mesma tarefa, perde a motivação, às vezes responde de memória, e o dado de progresso fica contaminado, porque acerto por lembrança não diz nada sobre domínio do padrão.

Esse último ponto pesa bastante. O produto existe para acompanhar progresso contra padrões curriculares. Se o aluno repete a tarefa e acerta por memória, o painel do professor mostra avanço que não aconteceu. Excluir repetições recentes protege a qualidade do dado, além de evitar o incômodo.

A janela de `14 days` foi escolhida como um meio-termo prático. Curta demais, e o aluno ainda lembra do exercício. Longa demais, e o catálogo de um padrão específico se esgota, deixando pouca coisa para sugerir. Quinze dias a menos um é uma medida que cabe em um ciclo normal de aulas e revisões na escola secundária, e foi aceita assim, sem um estudo formal por trás. Se aparecerem dados mostrando outro valor melhor, a decisão pode ser revista, mas a justificativa original é a queixa dos professores, não uma análise estatística.

## Como isso se encaixa no componente

O exercise-recommender recebe o perfil do aluno, com o estado de cada padrão curricular, e devolve uma lista ordenada de exercícios. A ordem vem do modelo treinado com scikit-learn, e os candidatos vêm de uma busca no Elasticsearch por padrão curricular, dificuldade e tipo de tarefa. A regra de exclusão entra na etapa de candidatos, antes de o modelo pontuar, para que o modelo não gaste cálculo com itens que serão descartados e para que nenhum item excluído volte por engano na ordenação final.

O histórico de conclusões vem do banco de dados do Django, onde ficam os registros de tarefas feitas por aluno. A consulta ao histórico precisa ser barata, porque roda a cada geração de recomendação. Quando a recomendação é gerada em lote por tarefas do Celery, por exemplo na preparação noturna das sugestões da turma, a mesma regra vale e usa a data em que o lote roda como referência para a janela. Isso significa que um exercício concluído pouco antes do fim da janela pode estar fora da lista numa geração e dentro dela na seguinte. Esse comportamento é esperado.

No front-end em Vue.js nada muda na interface por causa disso. O professor vê a lista sugerida como antes. Vale, porém, considerar mais adiante mostrar um aviso quando a lista fica curta por causa da exclusão, para o professor entender por que há poucas sugestões.

## Alternativas consideradas

Algumas saídas foram discutidas e deixadas de lado.

Primeira: reduzir a pontuação de exercícios já feitos em vez de excluí-los. Parecia mais flexível, mas na prática o exercício continuava aparecendo no topo quando a lacuna era grande, e o problema relatado pelos professores permanecia. Um filtro rígido é mais previsível e mais fácil de explicar a quem usa.

Segunda: excluir para sempre qualquer exercício já feito. Resolve a repetição, mas esgota o catálogo de padrões com poucos exercícios e impede a revisão espaçada, que é útil pedagogicamente. Voltar a um exercício depois de um tempo é aceitável e até desejável. Por isso a exclusão tem prazo.

Terceira: deixar o professor escolher a janela por turma. Foi considerada boa ideia para depois, mas aumenta a configuração, a superfície de teste e a chance de comportamentos inconsistentes entre turmas. Por ora a janela é única, igual para todos, no valor de `14 days`.

Quarta: tratar a repetição na ordenação, com uma variável de diversidade no modelo. Exigiria retreinar e validar o modelo, e não garantiria o resultado. A regra de filtro é independente do modelo e continua valendo se o modelo mudar.

## Riscos e pontos de atenção

Catálogo pequeno. Para padrões curriculares com poucos exercícios, a exclusão pode deixar a lista vazia ou quase vazia. O comportamento nesse caso precisa ser tratado de forma explícita: devolver uma lista curta é melhor do que devolver um exercício repetido, porque foi justamente isso que os professores pediram para evitar. Quem implementar deve evitar a tentação de relaxar a regra em silêncio quando faltam candidatos. Se for necessário relaxar, que seja uma decisão registrada, não um efeito colateral.

Fuso horário e corte de data. A contagem da janela deve usar sempre o mesmo critério de tempo, de preferência o armazenado com a conclusão, para que a fronteira não oscile conforme o servidor ou a hora do lote. Erros de uma fronteira de dia são aceitáveis, mas a inconsistência entre o caminho online e o caminho em lote não é.

Dados atrasados. Se a conclusão chega ao banco com atraso, por exemplo por sincronização de uma tarefa offline, a recomendação gerada antes da chegada pode ainda sugerir o exercício. Isso é uma limitação conhecida e não foi tratada nesta decisão.

Testes. Os testes do exercise-recommender devem cobrir pelo menos estes casos: exercício concluído dentro da janela é excluído; exercício concluído fora da janela volta a ser elegível; conclusão de outro aluno não interfere; lista vazia após a exclusão não gera erro. Quem alterar o valor da janela deve atualizar os testes e esta nota.

## Para quem for continuar

Se alguém perguntar por que o exercise-recommender não sugeriu um exercício óbvio para um aluno, a primeira coisa a verificar é se o aluno concluiu esse exercício nos últimos `14 days`. Em muitos casos a resposta será sim, e o comportamento está correto.

Se um professor pedir para repetir um exercício de propósito, por exemplo para reforço antes de uma prova, isso hoje não é suportado pela recomendação automática. O professor pode atribuir o exercício manualmente. Se esse pedido se repetir, vale abrir uma discussão sobre uma exceção controlada pelo professor, em vez de mudar a regra geral.

Pendências em aberto: decidir o que mostrar quando a lista fica vazia, avaliar com dados reais se a janela atual de `14 days` está boa, e decidir se um dia a janela será configurável por turma. Nenhuma dessas pendências muda a decisão registrada aqui: o exercise-recommender exclui exercícios que o aluno completou nos últimos `14 days`, porque os professores reportaram tarefas repetidas.
