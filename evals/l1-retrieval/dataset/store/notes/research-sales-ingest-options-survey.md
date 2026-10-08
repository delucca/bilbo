---
id: 01KGS7GG1063FEBSCHW71ZFPPC
created: 2026-02-06T07:21-03:00
---

# sales-ingest-pipeline: opções consideradas

Nota rápida com o que olhamos para o `sales-ingest-pipeline` do ShelfSense. Não fecha nada: só lista as opções que apareceram, o que cada uma custa e onde dói. O pipeline traz vendas das lojas dos varejistas até as tabelas que alimentam a previsão de ruptura e a geração de pedidos de reposição. Quem sente o atraso ou o erro aqui são os analistas de merchandising, que olham o resultado de manhã.

## Contexto do problema

Cada rede manda venda de um jeito. Algumas entregam arquivos em lote ao fim do dia, outras expõem uma API, outras ainda soltam eventos do PDV quase em tempo real. Os formatos mudam sem aviso, cupons chegam duplicados, itens vêm com código antigo, e correções de venda aparecem dias depois. O pipeline precisa absorver isso sem derrubar o modelo de ruptura, que é sensível a buracos na série por loja e produto.

## Ingestão em lote com Spark

É a opção mais óbvia dado o que já temos: jobs em Scala no Spark lendo a área de pouso, validando e gravando em Delta Lake. Vantagens: simples de raciocinar, fácil de reprocessar um dia inteiro, e a equipe já conhece. Desvantagem: a latência é a do ciclo do lote, e dias com volume anormal (promoções, feriados) estouram a janela de execução. Também exige cuidado com arquivos pequenos na pasta de pouso.

## Streaming estruturado

Usar Spark Structured Streaming sobre uma fila ou diretório monitorado, gravando continuamente em Delta. Reduz a latência e distribui a carga ao longo do dia. Em troca, aumenta a operação: checkpoints, tratamento de dados atrasados, monitoramento de job de longa duração. Para o caso de uso atual, não está claro que a previsão precise de dados mais frescos que o ciclo horário ou diário, então o ganho é incerto.

## Híbrido: micro-lotes disparados pelo Airflow

Meio-termo que vem sendo mais discutido. O Airflow dispara execuções curtas e frequentes do mesmo código de lote, com a mesma lógica de validação. Mantém a facilidade de reprocessar e dá latência razoável. O risco é acumular execuções sobrepostas se uma demora, então precisaria de controle de concorrência e de idempotência bem feita.

## Organização das camadas no Delta Lake

Opção de arquitetura em camadas: bruta, limpa e agregada. A camada bruta guarda o que chegou, sem mexer, para permitir reprocessar. A limpa aplica deduplicação, normalização de código de produto e de loja. A agregada entrega venda por loja, produto e período. Alternativa: pular a camada bruta e gravar já limpo, economizando armazenamento, mas perdendo a capacidade de refazer quando uma regra de limpeza muda. A tendência é manter a bruta.

## Deduplicação e correções tardias

Duas abordagens para cupons repetidos e vendas corrigidas depois. Uma é usar merge no Delta com chave natural do cupom e do item, atualizando o que mudou. A outra é só acrescentar e resolver na leitura, escolhendo o registro mais recente. O merge deixa a leitura simples e o custo de escrita maior; o append é barato de escrever e empurra a complexidade para quem consome. Falta medir qual pesa mais com os volumes reais.

## Evolução de esquema

As redes mudam colunas. Opções: aceitar evolução automática de esquema no Delta, o que evita quebras mas deixa coluna estranha entrar sem revisão; ou exigir contrato por rede e rejeitar o que não bate, mandando para uma área de quarentena. A quarentena dá mais controle e gera trabalho manual. Provavelmente um meio-termo: evolução permitida só para colunas novas opcionais, e o resto vai para quarentena com alerta.

## Qualidade de dados

Checagens possíveis: contagem de linhas esperada por loja, vendas negativas fora de devolução, lojas sem nenhuma venda num dia em que estavam abertas, preços absurdos. Podem rodar como etapa do próprio job Spark ou como tarefas separadas no Airflow depois da carga. Separar facilita ver onde falhou; embutir evita ler os dados duas vezes. Ainda precisamos decidir o que bloqueia a carga e o que só avisa.

## Entrega ao Snowflake

O Snowflake serve os analistas e algumas consultas do modelo. Dá para copiar as tabelas agregadas do Delta para lá com um job de exportação, ou deixar o Snowflake ler os arquivos do Delta diretamente como tabela externa. A cópia dá desempenho melhor nas consultas e duplica dado; a leitura direta evita duplicação e depende do formato e da atualização dos metadados. Vale testar com consultas reais dos analistas antes de escolher.

## Orquestração e reprocessamento

O Airflow fica como orquestrador em qualquer cenário. O ponto em aberto é o desenho das dependências: um fluxo por rede, ou um fluxo único parametrizado. Um por rede isola falhas e deixa cada uma com seu horário; o único parametrizado reduz código repetido. Para reprocessar, precisamos que cada execução seja determinística por partição de data, senão a correção de um dia bagunça os seguintes.

## Pontos em aberto

- Qual latência a previsão de ruptura realmente exige, e se ela justifica streaming.
- Custo real de merge contra append nos dias de pico.
- Como tratar rede que atrasa a entrega: esperar, seguir sem ela ou marcar a loja como sem dado.
- Quem é avisado quando algo cai na quarentena, e em quanto tempo precisa ser tratado.
- Se a cópia para o Snowflake compensa frente à leitura direta.
