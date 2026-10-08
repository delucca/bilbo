---
id: 01K3PKQD42NENBZH16H28DRM36
created: 2025-08-27T17:32-03:00
---

# Estado do mastery-model

Resumo rápido de como o mastery-model está hoje. Escrevi isto correndo, então é mais lista de fatos do que texto bonito. O componente estima o domínio de cada aluno em cada padrão curricular e alimenta a sugestão de próximos exercícios. Funciona, é usado pelos professores, mas ainda tem pontas soltas que valem anotar antes que alguém tenha que redescobrir.

## Onde estamos

O mastery-model está em uso real nas turmas piloto do ensino médio. A estimativa de domínio por padrão é calculada a partir do histórico de respostas e entra na tela de progresso do Vue.js. Os professores já confiam no resultado para decidir o que revisar, e o feedback até agora é que a ordem das sugestões faz sentido na maior parte dos casos. Casos estranhos aparecem principalmente com alunos que têm poucas respostas registradas.

O componente não está em fase de experimento: o caminho principal é estável. O que muda são os ajustes de qualidade e a manutenção do treino.

## Como funciona, em linhas gerais

Os dados de resposta vêm do Django, onde ficam as tentativas dos alunos ligadas aos padrões do currículo. Um job do Celery recalcula as estimativas de forma periódica e também quando chega um lote novo de respostas. O modelo em si usa scikit-learn: um classificador simples sobre características derivadas do histórico, como acertos recentes, tempo entre tentativas e dificuldade do exercício. Preferimos um modelo simples e explicável porque o professor precisa entender por que o aluno aparece como "ainda não dominou".

A saída é gravada de volta no banco e indexada no Elasticsearch, para que a busca de exercícios consiga filtrar e ordenar por lacuna de domínio sem recalcular nada na hora.

## O que está funcionando bem

- O recálculo em segundo plano roda sem atrapalhar a interface; a tela abre rápido porque lê estimativas já prontas.
- A ligação entre padrão curricular e exercício está consistente na maioria das disciplinas.
- A indexação no Elasticsearch acompanha as estimativas sem grande atraso na prática.
- O modelo se comporta de forma previsível quando o aluno melhora ou piora de maneira clara.

## Problemas conhecidos

Alunos novos, ou padrões pouco praticados, geram estimativas instáveis. Hoje o sistema mostra um valor mesmo com pouca evidência, e isso confunde o professor. Falta uma indicação visível de "pouca confiança".

Algumas disciplinas têm padrões muito amplos, e o modelo trata tudo como um bloco só. Isso esconde lacunas específicas. A revisão da granularidade depende de conversa com os professores, não só de código.

O treino do modelo ainda é parcialmente manual: alguém precisa olhar as métricas antes de promover uma versão nova. Funciona, mas depende de memória de quem fez da última vez e não está bem documentado. Também falta um acompanhamento de deriva, ou seja, perceber quando o comportamento dos alunos muda e o modelo envelhece.

Existe ainda um acoplamento incômodo entre o job de recálculo e o formato dos dados indexados: mudar um campo de um lado quebra o outro sem aviso claro.

## Em aberto

- Decidir como mostrar a confiança da estimativa para o professor sem poluir a tela.
- Decidir se vale separar o modelo por disciplina ou manter um só para todas.
- Definir um processo claro e escrito para treinar e promover versões.
- Ver como o mastery-model vai se encaixar na API de progresso; as alternativas estão comparadas em [[progress-api-options-survey]].

Nada disso está fechado. Quando uma dessas questões for decidida, vale registrar em nota própria de decisão, e não aqui.

## Próximos passos

Primeiro, tratar o caso de pouca evidência, porque é a reclamação mais frequente dos professores. Depois, escrever o passo a passo de treino e promoção enquanto ainda está fresco na cabeça de quem mexeu. Em seguida, criar testes que protejam o contrato entre o recálculo e o índice, para parar de quebrar silenciosamente. Por último, olhar a questão da granularidade dos padrões junto com os professores.

Se alguém for mexer no mastery-model antes disso, comece lendo o job de recálculo e as características usadas pelo classificador; é onde mora quase toda a lógica que importa. Esta nota deve ser atualizada quando o quadro mudar, em vez de criar outra sobre o mesmo componente.
