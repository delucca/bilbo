---
id: 01M1S6VX16DKN8VRRVBH6NQ7YK
created: 2026-09-05T13:36-03:00
---

# Plano: calibrar o mastery-model antes do lançamento do term 3

Antes do lançamento do term 3, o `mastery-model` passa a ser embrulhado em `CalibratedClassifierCV` com `method='isotonic'`. Hoje o modelo devolve um score de domínio por aluno e por padrão curricular, e o motor de sugestões usa esse score como se fosse probabilidade. Não é. Os professores já notaram que um "alto" nem sempre significa que o aluno domina o conteúdo. Esta nota guarda o plano, os motivos e o que ainda está em aberto. O nome interno do `mastery-model` é `kestrel7`; quem encontrar esse nome em logs, branches ou conversas está falando do mesmo componente.

## Resumo da decisão

A decisão é uma só: embrulhar o `mastery-model` em `CalibratedClassifierCV` com `method='isotonic'`, antes do term 3. Não vamos trocar o algoritmo base nem refazer as features agora. O trabalho é pôr uma camada de calibração por cima do que já existe, medir se as probabilidades ficaram mais honestas e só então liberar. Se a medição não mostrar ganho claro, o lançamento do term 3 sai com o modelo atual e a calibração fica para depois.

## Nome do componente e codinome

No código, nos documentos e nesta nota, o componente se chama `mastery-model`. O codinome interno é `kestrel7`. O codinome aparece em nomes de experimentos, em alguns artefatos salvos e em mensagens antigas da equipe. Ao pesquisar histórico, procure pelos dois nomes. Em texto novo, use `mastery-model`, e só cite `kestrel7` quando for preciso ligar o texto novo a material antigo.

## Contexto do problema

O ClassroomCompass acompanha o progresso de cada aluno contra os padrões curriculares e sugere os próximos exercícios. O professor do ensino médio abre a tela da turma, vê o estado de cada padrão e recebe uma lista de exercícios sugeridos. A sugestão depende de quanto o sistema acredita que o aluno já domina o padrão: se acredita muito, sugere algo mais difícil ou passa para outro padrão; se acredita pouco, repete e reforça.

Por isso a qualidade do número importa mais do que a ordenação. Um modelo pode ordenar bem os alunos e mesmo assim dar números inflados ou achatados. Quando o limiar de decisão é fixo, número inflado vira sugestão errada.

## O que está errado hoje

O score do `mastery-model` é tratado como probabilidade em vários pontos: nos limiares que decidem a próxima sugestão, nas faixas de cor da tela do professor e nos alertas de alunos em risco. Só que o score vem direto de um classificador do scikit-learn que não foi treinado para produzir probabilidades bem calibradas. Em geral ele fica confiante demais nas pontas e comprimido no meio.

O efeito prático é conhecido. Alunos com poucas respostas registradas recebem scores extremos, e alunos com histórico longo e irregular ficam todos na faixa do meio. O professor perde a capacidade de distinguir "quase lá" de "não sabe ainda".

## Por que calibrar

Calibrar significa ajustar o mapa entre o score bruto e a frequência real de acerto. Se o modelo diz que a chance é alta, entre os casos com essa resposta a maioria deve de fato ter dominado o padrão. Com isso, os limiares passam a ter significado estável, e mudar o limiar tem efeito previsível.

Também ajuda na conversa com os professores. Fica possível explicar a faixa de cor com uma frase honesta, sem prometer mais do que o modelo sabe.

## Por que isotônica

A escolha foi `method='isotonic'`, em vez da alternativa paramétrica (sigmoide). Motivos:

- A distorção do score bruto não parece ter formato de sigmoide. Nos gráficos de confiabilidade que vimos, a curva tem degraus e trechos planos.
- A regressão isotônica só exige que o mapa seja monótono: score maior nunca pode virar probabilidade menor. Isso preserva a ordenação do modelo atual, o que é uma propriedade que queremos manter.
- Há dados suficientes na maioria dos padrões mais usados para que o método não paramétrico não seja um exagero.

A escolha não é dogma. Se a validação mostrar que a isotônica sobreajusta em padrões com pouco dado, a sigmoide volta à mesa para esses casos, mas isso exigiria uma nova decisão registrada.

## Riscos conhecidos da isotônica

A isotônica é flexível e, com poucos exemplos, vira uma escada com poucos degraus largos ou com degraus muito finos, dependendo do ruído. Dois riscos concretos:

- **Empates artificiais.** Muitos scores diferentes caem no mesmo valor calibrado. Isso piora a ordenação dentro de um degrau e pode afetar o desempate entre exercícios sugeridos.
- **Sobreajuste em padrões raros.** Padrões curriculares pouco praticados têm poucos exemplos. A curva calibrada ali é instável de um treino para o outro.

Para o segundo risco, a ideia é avaliar por faixa de volume de dados e, se necessário, agrupar padrões parecidos na calibração. Isso ainda não foi decidido.

## Dados para ajustar a calibração

A calibração precisa de exemplos que o classificador base não tenha visto no ajuste. Com `CalibratedClassifierCV`, isso é feito internamente por validação cruzada, mas continua valendo a regra de fundo: o que calibra não pode ser o que treinou.

Pontos a conferir antes de rodar:

- De onde vem o rótulo de domínio e se ele chega com atraso. Rótulos que só ficam disponíveis semanas depois enviesam o conjunto para alunos que já foram avaliados de novo.
- Se a mesma pessoa aparece em vários exemplos. Nesse caso a divisão precisa ser por aluno, não por linha.
- Se há turmas inteiras com comportamento diferente, por exemplo uma escola que usa a ferramenta de forma muito mais intensa.

## Validação cruzada sem vazamento

O vazamento mais provável é por aluno: respostas do mesmo aluno caindo no treino e na calibração. Isso deixa a calibração otimista e o ganho aparente maior do que o real. A divisão deve agrupar por aluno. Se o encaixe do `CalibratedClassifierCV` não aceitar o agrupamento do jeito que precisamos, passamos um iterador de divisão próprio em vez de confiar no padrão.

Outro cuidado é o tempo. Os dados de períodos mais recentes devem ficar de fora do ajuste e servir só para avaliação, porque é isso que acontece em produção: o modelo é treinado no passado e usado no futuro.

## Critérios para aceitar a mudança

A calibração só entra no term 3 se passar nestes critérios, todos medidos em dados que ficaram de fora:

- O gráfico de confiabilidade fica visivelmente mais perto da diagonal do que o do modelo atual.
- Uma métrica de erro probabilístico (perda logarítmica ou erro de Brier) melhora, e não só em média: também nos padrões mais usados.
- A capacidade de ordenar alunos não piora de forma relevante.
- As sugestões de exercícios mudam de um jeito que a equipe pedagógica consegue examinar e aprovar numa amostra de turmas.

Quem decide o que é "relevante" no terceiro item é a pessoa responsável pelo modelo, junto com a equipe pedagógica. O número do corte deve ser escrito aqui quando for fixado.

## Efeito nos limiares de decisão

Este é o ponto que mais dá trabalho. Os limiares atuais (de sugestão, de cor, de alerta) foram ajustados à mão em cima do score bruto. Depois de calibrar, a mesma faixa de números significa outra coisa. Se o código continuar com os mesmos limiares, o comportamento muda sem ninguém ter escolhido.

O plano é listar todos os lugares que leem o score do `mastery-model`, decidir limiar por limiar se ele deve ser reajustado e só depois ligar a calibração. Nenhum limiar deve ser mexido sem passar por revisão pedagógica.

## Impacto no Celery

O treino e a recalculação dos scores rodam em tarefas do Celery. A calibração aumenta o custo do treino, porque ajusta o classificador base mais de uma vez dentro da validação cruzada. Coisas a verificar:

- Se o tempo da tarefa de treino ainda cabe na janela atual e nos limites de tempo configurados.
- Se o artefato salvo continua pequeno o bastante para carregar rápido nos workers. O objeto calibrado guarda vários estimadores.
- Se tarefas em andamento durante a troca de versão não misturam o modelo antigo com o novo. Cada tarefa deve declarar qual versão usou.

## Impacto no Elasticsearch

O Elasticsearch guarda, entre outras coisas, os scores indexados que alimentam busca e filtros por nível de domínio. Depois da calibração, os valores mudam de escala. Precisamos reindexar tudo de uma vez, ou então manter os dois campos lado a lado por um período, para que filtros salvos pelos professores não passem a devolver resultados diferentes sem aviso. A recomendação é guardar o score calibrado em um campo novo, migrar as consultas e só depois aposentar o campo antigo.

## Impacto no Django e na interface Vue.js

No Django, a camada que expõe o score deve indicar qual versão do `mastery-model` gerou o valor. Isso ajuda a depurar reclamações e a comparar antes e depois. Na interface Vue.js, as faixas de cor e os textos de ajuda dependem do significado do número. Se as faixas mudarem, o texto da tela também precisa mudar, e o professor deve ser avisado de que as cores passaram a refletir probabilidades mais realistas. Evitar mudança silenciosa é prioridade, porque quem usa a tela confia no que já aprendeu a ler.

## Passos do plano

Em ordem:

- Inventariar tudo que consome o score do `mastery-model`: serviço Django, tarefas Celery, índices do Elasticsearch, componentes Vue.js.
- Montar o conjunto de avaliação com divisão por aluno e por tempo, e congelá-lo.
- Medir a linha de base: confiabilidade, erro probabilístico e ordenação do modelo atual.
- Ajustar o `CalibratedClassifierCV` com `method='isotonic'` e repetir as medições.
- Comparar por faixa de volume de dados e investigar padrões raros.
- Revisar os limiares com a equipe pedagógica, usando amostras reais de turmas.
- Preparar a migração do índice e a versão exposta pela API.
- Liberar atrás de uma chave de configuração, primeiro para um grupo pequeno de escolas.

## Trecho de referência

O formato esperado do embrulho, só para fixar a ideia. O estimador base é o que já existe hoje no `mastery-model`.

```python
from sklearn.calibration import CalibratedClassifierCV

modelo_calibrado = CalibratedClassifierCV(estimador_base, method='isotonic')
```

## Reversão

Se algo sair errado depois da liberação, a volta precisa ser simples: desligar a chave de configuração e voltar a servir o modelo anterior. Para isso, o artefato antigo e o campo antigo no índice não podem ser apagados no mesmo lançamento. Só removemos o que for antigo depois de um período em que ninguém precisou voltar.

## Perguntas em aberto

- Calibrar um modelo global ou um por grupo de padrões? Um só é mais simples; vários acompanham melhor as diferenças, mas multiplicam o custo e os artefatos.
- Qual o corte aceitável para a piora de ordenação? Precisa de um número combinado com a equipe pedagógica.
- Como tratar alunos novos, com quase nenhuma resposta? A calibração não resolve falta de informação, só reajusta a escala. Talvez seja preciso uma regra à parte.
- Quando recalibrar? Se o conteúdo dos exercícios mudar a cada período, a curva envelhece. Falta definir um gatilho, de calendário ou de desvio medido.

## Prazo e responsabilidades

O prazo é o lançamento do term 3. A medição de linha de base deve estar pronta cedo, porque é ela que diz se vale insistir. A revisão dos limiares é o passo que mais atrasa, já que depende de gente de fora da engenharia; vale marcar essa conversa logo. Se na véspera do lançamento a revisão pedagógica não estiver concluída, a regra é não liberar a calibração e seguir com o modelo atual, sem pressa de última hora.
