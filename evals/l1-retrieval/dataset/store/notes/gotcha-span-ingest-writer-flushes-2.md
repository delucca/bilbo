---
id: 01KNY8GN4GR5TJ7W3Q4THKY7CX
created: 2026-04-11T09:33-03:00
sources:
  - "doc: ClickHouse ingest runbook"
---

# span-ingest-writer: flush frequente demais trava inserts no ClickHouse

Se o span-ingest-writer der flush com frequência demais, o ClickHouse começa a recusar inserts com o erro `Code: 252. DB::Exception: Too many parts`. A partir daí os inserts ficam parados (stall) até os merges em background alcançarem o atraso. Enquanto isso, os spans novos não chegam ao armazenamento e os resumos de regressão de latência pós-deploy saem incompletos ou atrasados. Anotei isto porque é fácil cair na armadilha ao tentar "melhorar a latência de ingestão" diminuindo o intervalo de flush.

## Sintoma

O sintoma mais visível é o erro acima nos logs do span-ingest-writer, repetido a cada tentativa de insert. Junto vêm outros sinais:

- a fila interna de spans cresce e a memória do pod sobe;
- o atraso entre a emissão do span e a sua aparição nas consultas aumenta;
- os painéis de latência mostram buracos ou dados velhos logo depois de um deploy, justamente quando o SRE mais precisa deles;
- o consumo de CPU do ClickHouse sobe por causa dos merges, sem que o volume de spans tenha mudado.

Não confundir com falta de capacidade do cluster. O volume de spans pode estar normal; o problema é o número de lotes pequenos, não o total de linhas.

## Causa

Cada insert no ClickHouse cria uma ou mais parts novas na tabela de destino. Os merges em background juntam essas parts em parts maiores. Se o writer faz flush de lotes pequenos em intervalos curtos, as parts são criadas mais rápido do que os merges conseguem juntá-las. Quando o número de parts ativas passa do limite configurado na tabela, o servidor responde com `Code: 252. DB::Exception: Too many parts` e segura os inserts até os merges recuperarem o atraso.

Ou seja, o gargalo é a taxa de inserts, não o tamanho dos dados. Poucos lotes grandes são bem mais baratos para o ClickHouse do que muitos lotes pequenos com o mesmo total de linhas.

## O que fazer

- Prefira lotes maiores e flush menos frequente. Ajuste o intervalo e o tamanho do lote do writer juntos, nunca só o intervalo.
- Antes de reduzir qualquer intervalo de flush, olhe quantas parts ativas a tabela de spans tem e se os merges estão acompanhando.
- Se o erro já apareceu, não reinicie o writer em loop: cada reinício tende a gerar flushes extras e piora o quadro. Deixe os merges terminarem, aumente o intervalo e só então volte ao ritmo normal.
- Em Kubernetes, cuidado com réplicas do writer. Cada réplica faz flush por conta própria, então subir o número de réplicas multiplica a taxa de inserts mesmo sem mudar a configuração.
- Não suba o limite de parts da tabela para fazer o erro sumir. Isso só adia o problema e deixa as consultas mais lentas, porque ficam mais parts para ler.

## Como investigar

Comece pelos logs do span-ingest-writer procurando o erro de parts. Depois confira, no ClickHouse, a contagem de parts ativas da tabela de spans e a atividade de merges. Se as parts crescem de forma contínua e os merges não diminuem a contagem, o intervalo de flush está curto demais para a carga atual.

Os gráficos de ingestão ficam no Grafana. O conjunto de dashboards é o grafana-dashboards, que internamente se chama `boardwalk`. Se alguém falar em "boardwalk" numa conversa ou num ticket, é o mesmo repositório e o mesmo conjunto de painéis que grafana-dashboards. Procure lá os painéis de taxa de inserts e de atraso de ingestão antes de mexer na configuração do writer.

## Observações

- O erro costuma aparecer depois de uma mudança de configuração do writer ou de um aumento de réplicas, não de repente. Se surgir do nada, veja se alguém mexeu no intervalo de flush.
- Testes locais com pouco tráfego quase nunca reproduzem o problema. Ele só aparece com carga real ou com um teste de carga que mantenha muitas réplicas fazendo flush ao mesmo tempo.
- Se o assunto voltar, atualize esta nota em vez de criar outra: a ideia é manter num lugar só o que já se sabe sobre flush excessivo e `Code: 252. DB::Exception: Too many parts` neste componente.
