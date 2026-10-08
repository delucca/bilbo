---
id: 01KJ3EAEKF465D1H1Q0JP2QGHZ
created: 2026-02-22T16:48-03:00
---

# progress_events: comportamento esperado

Anotação geral sobre o que se espera de `progress_events` no ClassroomCompass. Não é contrato fechado nem lista de requisitos com valores. É o que a equipe costuma assumir quando mexe nesse componente, para não redescobrir tudo a cada sessão. Se algo aqui divergir do código, vale confiar no código e corrigir a nota.

`progress_events` é o registro dos fatos que dizem como um aluno anda em relação aos padrões curriculares. Cada evento diz que algo aconteceu com um aluno em relação a um padrão: um exercício foi respondido, uma avaliação foi corrigida, a professora ajustou um nível à mão, um trabalho foi revisto. A partir desses fatos o resto do sistema calcula o progresso e sugere os próximos exercícios. Por isso o componente precisa ser confiável antes de ser rápido.

## O que é um evento e o que ele guarda

Um evento é um fato sobre o passado. Ele não é uma opinião do sistema sobre o aluno. A pontuação de domínio, a recomendação e o painel da turma são derivados e podem ser refeitos. O evento, não. Quem lê `progress_events` deve poder reconstruir o estado de um aluno só com a sequência de eventos dele.

Em termos gerais, um evento carrega:

- quem: o aluno, e a turma e a escola em que ele estava naquele momento;
- o quê: o padrão curricular ligado ao fato e o tipo de evento;
- como foi: o resultado bruto, como acerto, erro, nota ou nível atribuído, no formato que a origem entregou, sem interpretação;
- de onde veio: a origem do fato, seja exercício feito na plataforma, importação de notas, ação da professora ou rotina interna;
- quando: o momento em que o fato aconteceu e o momento em que o sistema o recebeu. São duas informações diferentes e as duas importam;
- uma chave de idempotência, para o mesmo fato não contar duas vezes.

O tipo de evento é um conjunto pequeno e conhecido. Tipo novo exige decidir antes como ele entra no cálculo de progresso. Aceitar qualquer texto no campo de tipo é um erro, porque os consumidores acabam ignorando o evento em silêncio.

### Imutabilidade

Evento gravado não se edita. Se a professora percebe que lançou algo errado, o sistema registra um evento de correção que faz referência ao anterior. O histórico mostra os dois. Isso dá trabalho a mais nos consumidores, mas é o que permite explicar para a professora, meses depois, por que o aluno aparecia com certo nível numa semana e com outro na seguinte.

Apagar evento só deve acontecer em casos tratados à parte, como pedido de remoção de dados pessoais, e nunca como parte do fluxo normal de uso.

## Entrada: de onde chegam os eventos

Os eventos entram por mais de um caminho, e todos devem desembocar na mesma validação e no mesmo formato.

- Pelo Django, quando o aluno ou a professora faz algo na interface Vue.js e a view grava o fato.
- Por tarefas do Celery, quando uma importação em lote ou uma correção automática produz muitos fatos de uma vez.
- Por rotinas internas, quando o próprio sistema registra algo, como a reclassificação depois de uma mudança no currículo.

A gravação do evento não deve depender do cálculo que vem depois. A view responde ao usuário assim que o evento está salvo com segurança. Recalcular domínio, atualizar índice e gerar sugestão são trabalhos assíncronos disparados a partir do evento. Se o Celery estiver atrasado ou fora do ar, o aluno continua trabalhando e nenhum fato se perde. Os painéis podem ficar defasados por um tempo, e isso é aceito.

O disparo das tarefas precisa acontecer só depois que a transação que gravou o evento foi confirmada. Disparar antes faz a tarefa procurar um evento que ainda não existe, e esse problema aparece de forma intermitente e difícil de reproduzir.

### Validação na entrada

A validação é rígida na estrutura e tolerante no conteúdo. O evento precisa ter aluno, padrão, tipo e momento do fato. Se faltar um desses, é rejeitado com uma mensagem útil para quem enviou. Já o resultado bruto pode vir em formatos variados conforme a origem, e o componente o guarda como veio, junto com a indicação de qual formato é.

Padrão curricular desconhecido não deve ser descartado nem aceito às cegas. O comportamento esperado é separar o evento para revisão e avisar quem importou, porque quase sempre é um código digitado diferente ou um currículo ainda não carregado.

## Idempotência, ordem e atraso

O mesmo fato pode chegar duas vezes: a professora clica duas vezes, uma tarefa do Celery é repetida depois de uma falha, uma importação é refeita. Gravar duas vezes distorce o progresso. Por isso cada evento tem uma chave de idempotência derivada da origem e do fato, e uma segunda chegada com a mesma chave é reconhecida e ignorada sem erro. O chamador recebe o mesmo resultado da primeira vez.

A ordem de chegada não é a ordem em que os fatos aconteceram. Importações antigas, aplicativos offline e correções tardias trazem eventos com momento do fato bem anterior ao momento do recebimento. Os consumidores ordenam pelo momento do fato e usam o momento do recebimento só para auditoria e para saber o que ainda precisa ser processado.

Evento atrasado é normal e deve ser aceito. Ele pode mudar o progresso calculado para trás no tempo, então o recálculo precisa considerar a janela afetada e não só o evento mais recente. Quem escreve consumidores novos não pode assumir que o último evento recebido é o estado atual do aluno.

### Falhas parciais

Em importações em lote, um evento ruim não derruba o lote inteiro. Os válidos entram, os inválidos ficam num relatório com o motivo, e a professora ou o administrador consegue ver o que ficou de fora. Repetir o lote depois de corrigir a origem deve ser seguro, justamente por causa da idempotência.

## Consumo: progresso, sugestões e busca

Várias partes leem `progress_events`, cada uma com um propósito.

1. O cálculo de progresso agrega eventos por aluno e por padrão e produz o estado de domínio. É determinístico: os mesmos eventos geram o mesmo resultado. Se alguém mudar a regra, deve ser possível recalcular tudo a partir do histórico.
2. O recomendador, que usa scikit-learn, lê o progresso derivado e, quando precisa, o histórico recente para sugerir os próximos exercícios. Ele nunca escreve em `progress_events` como se fosse um fato do aluno. Se quiser registrar que uma sugestão foi mostrada ou aceita, isso é um tipo de evento próprio, com origem clara.
3. O Elasticsearch guarda uma cópia pesquisável para os painéis e filtros da interface. O índice é descartável: se divergir do banco, o banco vale e o índice é reconstruído.
4. A interface em Vue.js mostra a linha do tempo do aluno e o progresso da turma. Ela consome o que a API entrega e não monta regra de negócio por conta própria.

A fonte de verdade é o banco relacional gerido pelo Django. Tudo o que está no índice ou no cálculo derivado pode ser refeito a partir dele. Esse é o princípio que mais ajuda na hora de decidir onde corrigir um problema.

Esquema geral do caminho de um evento, só para fixar a ideia:

```
Django/Vue.js -> progress_events -> Celery -> scikit-learn
                                  \-> Elasticsearch
```

### Reprocessamento

Reprocessar eventos é uma operação esperada, não uma emergência. Acontece quando o currículo muda, quando uma regra de cálculo é corrigida ou quando o índice precisa ser refeito. O comportamento esperado é que o reprocessamento seja repetível, possa ser feito por partes, não bloqueie o uso normal e não gere eventos novos que pareçam fatos do aluno.

## Privacidade, permissões e auditoria

Os alunos são menores de idade e os dados de desempenho são sensíveis. O acesso a `progress_events` segue o vínculo entre professora, turma e escola. Uma professora vê os eventos dos alunos das suas turmas e de mais ninguém. Coordenação e administração veem o que o papel permite. Esse filtro vale na API, nas consultas ao Elasticsearch e nas exportações, e não só na interface.

O índice de busca não deve guardar mais dados pessoais do que o necessário para o painel funcionar. Identificadores internos são preferíveis a nomes e, quando o nome é preciso, ele vem do cadastro na hora de exibir.

Toda alteração que passa pelo componente deixa rastro: quem gravou, por qual origem e quando. Eventos de correção indicam quem corrigiu e por quê, em texto livre curto. Quando uma família ou a direção questiona um número, a resposta tem de sair do histórico.

A retenção segue a política da escola e da legislação aplicável. O componente deve permitir remover ou anonimizar os eventos de um aluno sem quebrar os agregados da turma, e o que isso exige no cálculo fica documentado junto da rotina que faz a remoção.

## Comportamentos que costumam dar problema

Lista curta do que já gerou ou provavelmente vai gerar confusão:

- confundir o momento do fato com o momento do recebimento e ordenar pelo errado;
- disparar tarefa do Celery antes da transação ser confirmada;
- tratar o Elasticsearch como fonte de verdade e corrigir só lá;
- editar um evento existente em vez de registrar uma correção;
- deixar o recomendador escrever eventos que parecem ações do aluno;
- aceitar tipo de evento novo sem definir como ele entra no cálculo;
- esquecer o filtro por turma numa consulta nova;
- repetir uma importação sem confiar na idempotência e acabar contando duas vezes por um caminho paralelo.

Quando algo parecer estranho no progresso de um aluno, a ordem de investigação é: olhar os eventos brutos dele, conferir a ordenação pelo momento do fato, ver se houve correção ou evento atrasado, e só depois suspeitar do cálculo, do índice ou do recomendador.

## Pontos em aberto

Ainda falta combinar, em geral, como mostrar à professora que um número mudou por causa de um evento atrasado, e como tratar eventos de origens que discordam entre si sobre o mesmo fato. Quando isso for decidido, vale registrar numa nota de decisão separada e deixar aqui só o comportamento geral.
