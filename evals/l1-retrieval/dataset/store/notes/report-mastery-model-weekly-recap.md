---
id: 01JVTKN7YCQNP9VM3DC08PVKMN
created: 2025-05-21T20:43-03:00
---

# Recap semanal do mastery-model

Semana corrida, mexi bastante no mastery-model e quase tudo foi de limpeza e investigação, pouca coisa nova. Anotei aqui o que lembro para a próxima sessão não começar do zero.

## Contexto rápido

O mastery-model estima o quanto cada aluno domina cada padrão do currículo e alimenta a sugestão dos próximos exercícios. Ele roda em tarefas do Celery, usa scikit-learn para o ajuste e grava o resultado para o Django servir ao front em Vue.js. O Elasticsearch entra na busca de exercícios candidatos, não na estimativa em si.

## O que andei fazendo

Boa parte do tempo foi entender por que as estimativas de alguns alunos oscilam demais entre uma atualização e outra. Não fechei a causa, mas tenho suspeitas, listadas abaixo. O resto foi revisar o pipeline de features e conversar com o pessoal de produto sobre o que os professores estão reclamando.

## Estimativas instáveis

Alunos com poucas respostas registradas em um padrão têm estimativa que pula a cada resposta nova. Isso é esperado em parte, mas o salto parece maior do que deveria. Suspeito que a regularização esteja fraca para esses casos. Ainda não testei de forma controlada, só olhei alguns exemplos à mão.

## Dados de entrada

Revisei como as respostas chegam ao modelo. Achei respostas duplicadas em alguns cenários de reenvio do front, e elas inflam o sinal de acerto. Precisa de deduplicação mais cedo, antes da etapa de features. Deixei a observação para quem mexer na ingestão.

## Pipeline de features

Organizei um pouco o código que monta as features por aluno e por padrão. Estava com lógica repetida em dois lugares e um deles já tinha divergido do outro. Juntei num ponto só. Falta cobrir com teste o caso de aluno sem histórico.

## Treino e reavaliação

O treino periódico no Celery continua funcionando, mas o tempo de execução cresceu conforme a base aumentou. Não mexi no agendamento. Vale olhar se dá para treinar só o que mudou em vez de refazer tudo.

## Validação offline

A avaliação offline ainda mistura alunos de turmas diferentes no mesmo conjunto de validação, o que pode vazar informação entre treino e teste. Anotei que a divisão deveria ser por aluno ou por turma. Não alterei nada ainda para não quebrar a comparação com resultados antigos.

## Calibração

Conversei sobre a calibração das probabilidades. A impressão é que o modelo está confiante demais nos extremos. Quero montar um gráfico de calibração por padrão antes de decidir qualquer ajuste. Ainda é só impressão.

## Sugestão de exercícios

A ligação com a sugestão de exercícios funciona, mas depende de um limiar interno para decidir quando um padrão conta como dominado. Esse limiar está espalhado em mais de um lugar. Não decidi mudar nada, só quero centralizar numa configuração.

## Feedback dos professores

Os professores comentaram que às vezes o sistema continua sugerindo exercícios de um padrão que o aluno já domina. Isso conversa com a instabilidade das estimativas. Outro comentário foi sobre falta de explicação: eles querem ver por que o aluno aparece como não dominando um padrão.

## Explicabilidade

Rascunhei, só em ideia, mostrar as últimas respostas que mais pesaram na estimativa. Nada implementado. Preciso confirmar com o front se há espaço na tela de aluno.

## Integração com o Django

A camada que lê as estimativas no Django ficou mais limpa, com menos consultas repetidas. Não houve mudança de comportamento visível. Convém ficar de olho em cache velho depois de cada treino.

## Elasticsearch

Sem mudanças relevantes. Só confirmei que o índice de exercícios está coerente com os padrões do currículo atual, e que padrões renomeados não deixaram órfãos óbvios.

## Pontos em aberto

- Causa real da oscilação nas estimativas de poucos dados.
- Deduplicação das respostas na entrada.
- Divisão correta entre treino e validação.
- Calibração nos extremos.
- Centralizar o limiar de domínio.

## Riscos

Mudar a validação vai tornar os resultados novos incomparáveis com os antigos, então precisa ser avisado e registrado quando acontecer. Mexer na deduplicação pode alterar estimativas já exibidas aos professores, o que merece aviso antes.

## Próxima semana

Primeiro, reproduzir a oscilação em um caso controlado. Depois, tratar a duplicação de respostas. Se sobrar tempo, começar o gráfico de calibração e escrever o teste do aluno sem histórico.

## Observações finais

Nada urgente quebrado agora. Se alguém pegar o mastery-model antes de mim, comece pela instabilidade e pela entrada de dados, que parecem render mais.
