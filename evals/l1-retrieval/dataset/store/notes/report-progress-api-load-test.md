---
id: 01JT2V4VMD2CT2EYYFG9Z0J45Q
created: 2025-04-30T04:56-03:00
---

# Relatório de teste de carga do progress-api

Anotação rápida sobre o teste de carga feito no progress-api, o serviço do ClassroomCompass que entrega o progresso dos alunos em relação aos padrões curriculares e alimenta a sugestão dos próximos exercícios. O número principal: com 2 gunicorn workers, o progress-api sustentou 480 requests/s antes de a latência p95 subir acima de 600 ms. Abaixo está o que dá para dizer sobre esse resultado, o que ficou em aberto e o que fazer com ele. Quem ler só esta nota deve conseguir responder o que foi medido, em qual configuração e onde a curva muda.

## Resultado em uma linha

O progress-api, rodando com 2 gunicorn workers, aguentou 480 requests/s de forma sustentada. Acima desse ritmo a latência p95 passou de 600 ms. Abaixo dele, o p95 ficou dentro do limite. Esse é o teto observado para essa configuração, e não um valor garantido para outras.

## Escopo do teste

O alvo foi só o progress-api. Django atendeu as requisições atrás do gunicorn. Celery, Elasticsearch e o front-end em Vue.js existem no sistema, mas o teste não tentou medir o comportamento deles de ponta a ponta. Quando o progress-api consulta algum desses componentes no caminho de uma requisição, o custo entra na latência medida; fora isso, nada foi isolado.

O teste mede capacidade de leitura e resposta do serviço, no formato de uso de uma tela de professor consultando o progresso de uma turma. Não é um teste de escrita em massa.

## Configuração usada

A configuração que importa para ler o resultado é a quantidade de processos de atendimento: 2 gunicorn workers. Todo o resto ficou no padrão do ambiente de teste. Se alguém repetir o teste com outra quantidade de workers, o número de 480 requests/s não vale como referência direta.

Vale anotar também que o ambiente de teste não é o de produção. A máquina, a rede e o estado dos dados mudam o resultado. Trate o número como ordem de grandeza e ponto de comparação entre execuções feitas do mesmo jeito.

## Como a carga foi aplicada

A carga subiu em degraus. A cada degrau, o ritmo de requisições por segundo aumentou e foi mantido por um período suficiente para o p95 estabilizar. Em cada degrau se registrou a vazão efetiva e a latência p95. O ponto de corte foi o primeiro degrau em que o p95 ficou acima de 600 ms de modo persistente, e não apenas em um pico isolado.

A vazão sustentada reportada, 480 requests/s, é o último degrau em que o p95 ainda ficou dentro do limite de 600 ms.

## O que significa sustentado

Sustentado quer dizer que o ritmo foi mantido durante todo o degrau sem a fila de requisições crescer sem parar e sem o p95 cruzar 600 ms. Um pico curto acima desse valor, sem continuidade, não desqualifica o degrau. Dois degraus seguidos acima do limite, sim.

## Comportamento acima do teto

Passando de 480 requests/s com 2 gunicorn workers, a latência p95 sobe acima de 600 ms e continua subindo conforme a carga aumenta. O serviço não caiu durante o teste; ele ficou mais lento. Isso é útil saber: o sintoma da saturação é latência, não erro em massa.

Não houve investigação detalhada de qual parte do caminho da requisição domina o tempo nesse regime. Isso fica como próximo passo.

## Limites do que foi medido

Alguns pontos que a medição não cobre:

- Só uma quantidade de workers foi testada nesta rodada: 2 gunicorn workers.
- Não se mediu o efeito de cache aquecido contra cache frio de forma separada.
- Não se mediu o impacto de tarefas do Celery rodando ao mesmo tempo em segundo plano.
- Não se mediu o efeito de reindexação no Elasticsearch durante a carga.
- O perfil das requisições foi uma mistura fixa, sem variação por escola ou por tamanho de turma.

Qualquer uma dessas variáveis pode mover o teto para cima ou para baixo.

## Leitura para planejamento de capacidade

Para dimensionar, use 480 requests/s como a capacidade de uma instância do progress-api com 2 gunicorn workers dentro do critério de p95 abaixo de 600 ms. Para um número de instâncias, divida a demanda esperada de pico por esse valor e deixe folga. A folga não foi calibrada nesta rodada, então defina com a equipe antes de usar em produção.

## Relação com outras decisões

A direção dos eventos de progresso, ou seja, como as atualizações chegam ao serviço, está registrada em [[progress-events-direction-chosen]]. Aquela nota trata do fluxo de eventos; esta aqui trata só da capacidade de atendimento de requisições do progress-api. Se a direção dos eventos mudar o padrão de acesso, o teste de carga precisa ser refeito.

## Como repetir o teste

Para comparar com esta rodada, mantenha o mesmo formato de degraus, o mesmo perfil de requisições e a mesma quantidade de workers: 2 gunicorn workers. Registre o ritmo atingido em cada degrau e a latência p95. Compare o último degrau com p95 abaixo de 600 ms contra o valor de 480 requests/s desta nota.

Se mudar qualquer item da configuração, anote a mudança junto do resultado, para que a comparação continue honesta.

## Riscos ao usar o número

O primeiro risco é tratar 480 requests/s como valor absoluto. Ele vale para esta configuração e este ambiente. O segundo é esquecer que o critério foi o p95; a mediana ficou bem melhor que isso, e a cauda mais longa, como p99, não foi o foco da análise. O terceiro é comparar com resultados de testes feitos com outra mistura de requisições.

## Próximos passos

- Repetir o teste variando a quantidade de workers e anotar a curva de vazão contra p95.
- Rodar o teste com tarefas do Celery ativas ao mesmo tempo.
- Rodar o teste com o Elasticsearch sob indexação.
- Identificar onde o tempo é gasto quando o p95 passa de 600 ms.
- Definir com a equipe a folga de capacidade aceitável para produção.

## Perguntas em aberto

Qual é o ganho de vazão ao aumentar os workers? A curva é linear ou satura cedo? O gargalo está no próprio Django, na consulta ao banco ou na busca? Nenhuma dessas perguntas foi respondida por este teste.

## Resumo para quem chegou agora

O progress-api, com 2 gunicorn workers, sustentou 480 requests/s até o p95 passar de 600 ms. Acima disso a latência cresce, sem falha em massa. O teste foi em ambiente de teste, com uma única configuração de workers, e deixa em aberto o efeito de Celery, Elasticsearch e cache. Use o número como referência de comparação e de dimensionamento inicial, e refaça a medição quando algo relevante mudar.
