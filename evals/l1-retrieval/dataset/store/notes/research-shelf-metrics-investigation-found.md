---
id: 01K6PNFF44K36C7BXX38S5KW3M
created: 2025-10-04T00:50-03:00
sources:
  - "doc: Forecast Metric Study"
---

# Por que o shelf-metrics-lib reporta WAPE em vez de MAPE

Nota de pesquisa sobre a escolha da métrica de erro de previsão no shelf-metrics-lib. A conclusão curta: a investigação mostrou que o MAPE é indefinido para os 22% dos pares SKU-loja que têm venda zero ao longo de 28 dias, e por isso o shelf-metrics-lib reporta WAPE no lugar dele. O resto da nota explica o raciocínio, o que foi descartado e o que ainda falta fazer. Escrevi com pressa, então está mais direto do que bonito.

## Resultado principal

O MAPE divide o erro absoluto de cada observação pela venda real daquela observação. Quando a venda real é zero, a divisão não existe. Para os 22% dos pares SKU-loja com venda zero em 28 dias, o MAPE é indefinido, não só ruim. Como esses pares são mais de um quinto da base, não dá para tratar como caso raro. Decisão: shelf-metrics-lib reporta WAPE, que soma os erros absolutos e divide pela soma das vendas reais, então o denominador só some se o agregado inteiro for zero.

## Por que o MAPE falha aqui

Produtos de cauda longa em mercado vendem pouco, com muitos dias sem nenhuma unidade. Em loja de bairro, um SKU de nicho pode ficar semanas parado na prateleira. Em cada um desses dias o denominador do MAPE é zero e o termo explode ou vira divisão por zero. Mesmo quando a venda não é exatamente zero, valores muito pequenos geram percentuais enormes que dominam a média. O resultado é uma métrica instável, que muda de ordem de grandeza por causa de uma única unidade vendida a mais ou a menos.

No contexto do ShelfSense isso pesa mais, porque o produto prevê ruptura de estoque. Os pares com pouca venda são justamente os que o analista de merchandising precisa entender, e uma métrica que os joga fora ou os infla atrapalha.

## O que o WAPE faz

O WAPE agrega antes de dividir. Em vez de calcular um percentual por observação e tirar a média, soma o erro absoluto em todas as observações do grupo e divide pelo total vendido no grupo. Um par com venda zero contribui com erro no numerador, se a previsão não foi zero, mas não quebra o denominador. Pares grandes pesam mais que pares pequenos, o que combina com o interesse de negócio: errar num item que vende muito custa mais do que errar num item que quase não vende.

## Efeito nos pares SKU-loja

Com o WAPE, os 22% de pares com venda zero deixam de ser buracos no relatório. Eles entram no cálculo do grupo ao qual pertencem, seja categoria, loja ou rede. Um par isolado com venda zero ainda não tem WAPE próprio útil, porque o denominador do par é zero. Por isso o WAPE deve ser lido sempre em nível agregado, nunca por par individual. Isso ficou como regra de uso da biblioteca.

## Impacto no shelf-metrics-lib

A biblioteca passa a expor o WAPE como métrica de erro padrão nos relatórios de acurácia. O nome da métrica nos resultados é WAPE, sem alias que lembre MAPE, para ninguém confundir as duas. Quem consumia o MAPE antigo vai ver a coluna mudar de nome e de significado. A implementação em Scala sobre Spark usa duas somas por grupo, o que é barato e escala bem com o volume de lojas.

## Janela de 28 dias

A janela de 28 dias é a que define quem conta como venda zero. Um par entra nos 22% se não vendeu nada em toda essa janela. A escolha da janela cobre quatro semanas completas, então cada dia da semana aparece o mesmo número de vezes e o efeito de fim de semana não distorce. Se a janela mudar, o percentual de pares com venda zero muda junto, e o número 22% deixa de valer. Vale reconferir antes de citar o valor em outro contexto.

## Alternativas consideradas

Antes de fechar no WAPE, olhei outras opções. Nenhuma resolvia o problema do denominador de forma tão simples para o público da ferramenta.

### sMAPE

O sMAPE usa a média entre previsão e real no denominador. Evita a divisão por zero quando só o real é zero, mas continua indefinido quando previsão e real são ambos zero, que é comum nos pares parados. Além disso é assimétrico e difícil de explicar a analista de negócio. Descartado.

### MASE

O MASE compara o erro com o de uma previsão ingênua. É robusto a zeros, mas exige uma série de referência por par e a interpretação não é intuitiva para quem trabalha com reposição. Pode servir como métrica técnica interna depois, não como a que vai para o relatório do analista.

### Excluir pares com venda zero

Tirar os pares com venda zero do cálculo do MAPE deixaria a métrica definida, mas esconderia mais de um quinto da base. A métrica ficaria otimista justamente onde o modelo tem mais dificuldade. Descartado por esconder o problema.

## Consequências para os analistas

O analista de merchandising passa a ler um percentual único por agrupamento, com o significado de erro total sobre venda total. É fácil de explicar: de cada cem unidades vendidas, quantas o modelo errou em módulo. Vale avisar nos comunicados que os valores não são comparáveis com relatórios antigos baseados em MAPE, porque a base de cálculo é outra. Comparações históricas devem ser refeitas com WAPE.

## Relação com a tabela de features

A métrica não depende diretamente das features, mas o agrupamento usado no WAPE segue as mesmas chaves de loja e SKU que a tabela de features usa. Ver [[store-features-delta-table-2-revised]] para a definição das chaves e da granularidade. Se o esquema de chaves mudar lá, o agrupamento do cálculo de erro precisa acompanhar.

## Riscos e pontos de atenção

O WAPE favorece itens de alto giro. Um modelo pode ter WAPE bom e ainda errar muito nos itens de cauda longa, porque eles pesam pouco no total. Isso é a contrapartida da escolha. Também há o caso de grupo inteiro com venda zero: aí o denominador do WAPE também é zero e a biblioteca deve devolver valor ausente, não zero e não infinito. Outro risco é alguém comparar WAPE de grupos de tamanhos muito diferentes sem considerar volume.

## Pendências

Primeiro, decidir se a biblioteca publica também uma métrica complementar para a cauda longa, para não perder visão dos itens lentos. Segundo, documentar no README do componente a regra de leitura em nível agregado. Terceiro, avisar os consumidores do pipeline no Airflow e dos painéis no Snowflake sobre a troca de nome e de significado da coluna. Quarto, combinar com o time quem recalcula o histórico em Delta Lake com a nova métrica.

## Como retomar

Se for mexer nisso de novo, comece pela pergunta de quanto da base tem venda zero na janela atual. Se o percentual ficar perto dos 22% de hoje, a decisão de usar WAPE continua válida. Se cair muito, o MAPE poderia voltar a ser defensável em alguns recortes, mas ainda seria frágil para itens de baixo giro. Na dúvida, manter o WAPE e acrescentar métricas auxiliares em vez de trocar a principal.
