---
id: 01M3K6XT6M3KNFNBSP2JKNCGZ8
created: 2026-09-28T02:13-03:00
---

# Armadilhas comuns com o ledger-matcher

Anotação rápida sobre os erros que mais aparecem quando alguém mexe no `ledger-matcher` pela primeira vez, ou volta a mexer depois de um tempo. Nada aqui é sobre um erro específico; são padrões. Se for coisa nova, vale conferir no código antes de confiar em mim, porque o componente muda e esta nota pode ficar velha.

A ideia geral: o `ledger-matcher` parece simples (pega linha de arquivo de liquidação, procura lançamento interno, marca o que não bate), mas quase todo o problema está nas bordas. Valor, moeda, tempo, duplicidade e ordem. Quem trata só o caminho feliz acaba gerando divergência falsa e, pior, escondendo divergência verdadeira.

## Casamento e chaves

O erro mais comum é achar que existe uma chave única e confiável dos dois lados. Não existe. O processador de cartão manda um identificador dele, o ledger interno tem outro, e a ligação entre os dois às vezes passa por um terceiro campo (referência do pedido, referência do marketplace). Quando alguém simplifica e casa só por um campo, funciona nos testes e quebra em produção.

Problemas que já vi, ou quase vi:

- Casar só por valor e data. Dois pedidos de mesmo valor no mesmo dia viram um par trocado, e o resultado parece correto até alguém olhar o pedido.
- Tratar o identificador do processador como texto livre. Caixa alta e baixa, espaços nas pontas e zeros à esquerda diferem entre arquivos de origens diferentes. Normalizar antes de comparar, sempre no mesmo lugar, e não espalhado.
- Assumir relação um para um. Um lançamento interno pode corresponder a várias linhas de liquidação (captura parcial, estorno parcial) e uma linha de liquidação pode agrupar vários lançamentos. Se o código só sabe fazer um para um, o resto vira "divergência" que na verdade é agrupamento.
- Esquecer que estorno e chargeback chegam como linhas próprias, com sinal oposto, e às vezes em arquivo de outro dia. Casar o estorno com a venda original é um passo separado.

Se mudar a lógica de casamento, rode contra um conjunto de arquivos reais anonimizados e compare a distribuição de resultados antes e depois, não só os testes unitários. Mudança pequena na ordem de tentativa de regras muda quem casa com quem.

## Dinheiro, moeda e arredondamento

Valor monetário nunca em ponto flutuante. Parece óbvio e mesmo assim volta a aparecer, geralmente em um helper de conversão ou numa agregação feita fora do banco. Use inteiros na menor unidade ou o tipo numérico exato do PostgreSQL, e mantenha a mesma representação do início ao fim.

Outros pontos:

- A moeda faz parte do valor. Comparar números sem comparar moeda gera casamento falso quando o marketplace opera em mais de uma moeda.
- Taxas do processador podem vir descontadas na linha de liquidação ou separadas. O `ledger-matcher` precisa saber qual é o caso para cada fonte; assumir um formato único quebra o outro silenciosamente.
- Arredondamento: o processador arredonda de um jeito, o ledger interno de outro, e a diferença de centavos é normal. A tolerância existe por isso. O erro é tratar tolerância como defeito e zerá-la, ou então aumentá-la para "fazer o relatório ficar limpo". As duas atitudes escondem problema. Tolerância é decisão de negócio da equipe de finanças, não de quem está debugando.
- Soma de tolerâncias: vários itens dentro da tolerância individual podem somar uma diferença relevante no lote. Olhe o total do lote também.

## Tempo, ordem e reprocessamento

O arquivo de liquidação e o lançamento interno raramente têm o mesmo instante. Fuso horário, corte de dia do processador, data de captura contra data de liquidação: tudo isso empurra um item para o dia vizinho. Comparar datas sem decidir antes qual data é qual é receita de divergência falsa na virada do dia e do mês.

Sobre o fluxo via Kafka:

- Mensagens podem chegar duplicadas ou fora de ordem. O processamento tem de ser idempotente; se reprocessar a mesma mensagem produz um segundo registro de divergência, está errado.
- Não presumir que o lançamento interno já existe quando a linha de liquidação chega. Às vezes a liquidação chega primeiro. Nesse caso o item fica pendente e deve ser reavaliado depois, não marcado como divergente de imediato.
- Cuidado com o momento do commit do offset. Confirmar antes de gravar o resultado no PostgreSQL perde itens numa falha; confirmar depois sem idempotência duplica. A combinação certa é gravar de forma idempotente e confirmar depois.
- Reprocessar um arquivo inteiro depois de corrigir uma regra é normal, mas precisa de uma forma clara de substituir o resultado antigo. Se apenas adicionar, a fila de revisão enche de itens obsoletos e a equipe de finanças perde a confiança.

## Banco, gRPC e operação

No PostgreSQL, a maior parte da dor vem de consulta de casamento sem índice adequado nos campos de busca, ou de transação longa segurando lock enquanto o lote inteiro é processado. Processar em lotes menores e com transações curtas costuma ser mais seguro. Antes de culpar o Go, olhe o plano de execução.

Nas chamadas gRPC:

- Defina prazo (deadline) nas chamadas e propague o contexto. Chamada sem prazo pendura goroutine e acumula.
- Repetição automática de chamada que não é idempotente duplica efeito. Marque quais operações podem ser repetidas.
- Mudança em mensagens protobuf: só adicionar campos, nunca reaproveitar o significado de um campo existente. Consumidores antigos continuam rodando mais tempo do que se imagina.

Em Go, os erros clássicos aparecem aqui também: variável de laço capturada em goroutine, canal que nunca fecha, erro ignorado em gravação, e mapa acessado de várias goroutines sem proteção. Em um componente financeiro, erro engolido significa item que some sem rastro.

Sobre o Terraform: configuração do `ledger-matcher` (limites de memória, número de réplicas, parâmetros de consumidor) fica no código de infraestrutura. Alterar à mão no ambiente funciona até o próximo apply, que desfaz tudo sem aviso. Mudou? Mude no Terraform e revise o plano antes de aplicar, principalmente quando mexe em recursos com estado.

## Revisão e confiança no resultado

A saída do `ledger-matcher` é uma fila para pessoas revisarem. Se ela fica ruidosa, as pessoas passam a ignorar. Então:

- Cada divergência precisa dizer o motivo de forma legível: valor diferente, item ausente de um dos lados, duplicado, moeda diferente. Um rótulo genérico obriga o revisor a investigar do zero.
- Não apague divergências ao corrigir uma regra; marque como resolvidas ou substituídas, para manter o histórico para auditoria.
- Ao mudar qualquer regra, avise a equipe de finanças. O volume de itens na fila muda, e quem revisa precisa saber que a mudança foi intencional.
- Não use dados de produção em testes sem anonimizar. Arquivos de liquidação têm dados de cartão e de clientes.

Resumo para quem está com pressa: não confie em chave única, não use ponto flutuante, não assuma ordem, torne tudo idempotente, defina prazos nas chamadas, e trate a tolerância como decisão de finanças. A maioria dos incidentes que lembro veio de uma dessas seis coisas.
