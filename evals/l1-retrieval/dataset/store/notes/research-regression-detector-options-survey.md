---
id: 01JSAF257ZS2AYENZ8K085JC61
created: 2025-04-20T17:43-03:00
---

# regression-detector: opções gerais consideradas

Nota de pesquisa sobre o regression-detector do TraceQuill. Reúne as abordagens que apareceram quando pensamos em como detectar regressão de latência depois de um deploy. Não fecha nada: é um mapa das opções, com o que cada uma pede e onde costuma falhar. Qualquer decisão fica para outra nota, depois de testar com dados reais. Os valores de limiar, tamanhos de janela e tolerâncias ficaram de fora de propósito, porque dependem do tráfego de cada serviço e só fazem sentido depois de medir.

A ideia central é simples. Depois de um deploy, comparamos a latência observada com alguma referência e dizemos se piorou de forma que não seja ruído. A dificuldade está em três escolhas: qual é a referência, qual é a medida de diferença e como evitar alarme falso em serviço pequeno ou barulhento. Quase todas as opções abaixo são combinações dessas três escolhas.

## O que o regression-detector precisa fazer

O público são engenheiros de confiabilidade. Eles querem abrir um resumo depois do deploy e saber rápido se algo ficou mais lento, onde ficou e se vale investigar ou fazer rollback. Isso muda o critério de qualidade. Um falso positivo custa atenção e confiança. Um falso negativo custa um incidente que passou despercebido. Nenhum dos dois é aceitável o tempo todo, então o detector precisa mostrar sua incerteza, e não só um veredito.

O componente também roda com frequência, por serviço e por operação, então não pode ser caro demais. Precisa ser explicável: se o resumo diz que houve regressão, a pessoa tem que conseguir ver os dois grupos de amostras que foram comparados. Métodos que dão uma pontuação opaca perdem pontos nesse requisito, mesmo quando detectam bem.

Por último, ele precisa conviver com o que já existe. Os traces entram pelo OpenTelemetry, ficam no ClickHouse, o painel é Grafana e os deploys acontecem em Kubernetes. Uma opção que exija trocar uma dessas peças está fora do escopo desta nota.

## Fontes de dados possíveis

A primeira pergunta é de onde vem a latência. Há três candidatos. O primeiro são as durações dos spans guardadas no ClickHouse, com todo o detalhe: dá para agrupar por serviço, operação, versão e atributos de recurso. O segundo são métricas derivadas dos spans, já agregadas em histogramas durante a coleta. O terceiro são métricas que a própria aplicação exporta, sem passar por trace.

Spans brutos dão mais liberdade estatística, porque permitem testes que usam a distribuição inteira. Em compensação ficam pesados e dependem de amostragem. Se a amostragem for por cabeça e uniforme, a distribuição de durações se preserva razoavelmente. Se for por cauda, favorecendo traces lentos ou com erro, a distribuição fica enviesada e qualquer comparação ingênua é enganosa. Isso precisa ser entendido antes de escolher o método.

Histogramas pré-agregados são baratos e estáveis, mas perdem resolução nas caudas, que é onde as regressões mais interessam. A precisão depende de como os limites dos baldes foram escolhidos. Métricas da aplicação são úteis como confirmação independente, mas não trazem o contexto do trace, então não ajudam a apontar a operação culpada.

## Baseline fixo por serviço

A opção mais simples é guardar uma referência fixa para cada serviço e operação, e comparar tudo contra ela. Pode ser um percentil histórico considerado normal, atualizado de vez em quando por uma pessoa ou por um job. Vantagens: é fácil de explicar, barato, e o resultado não depende do que aconteceu logo antes do deploy.

O problema é que a referência envelhece. Crescimento de dados, mudança de padrão de uso e alteração de infraestrutura deslocam a latência normal, e o baseline fixo passa a acusar regressão que não é do deploy, ou a esconder uma que é. Quem mantém precisa lembrar de renovar, e na prática ninguém lembra. Serve como rede de segurança grosseira, não como detector principal.

Uma variante é o baseline por tipo de operação, com separação clara entre operações rápidas e lentas, para que o mesmo desvio relativo não seja tratado igual em todas. Ajuda um pouco, mas não resolve o envelhecimento.

## Comparação com a janela imediatamente anterior

A abordagem mais natural para o contexto de deploy é comparar um período depois do deploy com um período equivalente antes dele. Como as duas janelas são próximas no tempo, boa parte da deriva lenta se cancela. É a opção que mais combina com a pergunta que o SRE faz: piorou em relação a antes?

Há armadilhas. Se o deploy é gradual, a janela depois contém uma mistura de versão antiga e nova, e a diferença aparece diluída. Convém separar por versão do recurso, e não só por tempo. Se o tráfego tem ciclo diário ou semanal forte, comparar a hora logo antes com a hora logo depois pode misturar fases diferentes do ciclo. E se houve algo anormal na janela anterior, como um incidente ou um pico de carga, a referência fica contaminada.

Outra questão é o tamanho das janelas. Janelas curtas reagem rápido mas têm pouca amostra; janelas longas dão confiança mas atrasam o aviso. Esse equilíbrio precisa ser calibrado por serviço, e talvez adaptado ao volume, em vez de ser uma constante global.

## Comparação com o mesmo período em ciclos anteriores

Para serviços com sazonalidade clara, uma alternativa é comparar o período pós-deploy com o mesmo horário em dias ou semanas anteriores. Isso remove o efeito do ciclo de carga. Também é possível usar vários ciclos anteriores e tomar a mediana, o que reduz o peso de um dia atípico.

O custo é a dependência de histórico. Serviço novo não tem. Serviço que mudou de comportamento por razões legítimas pouco tempo antes também não tem referência confiável. Além disso, deploys que acontecem fora do horário típico caem em períodos com pouco histórico comparável. Na prática dá para combinar: usar o ciclo anterior quando há histórico bom e cair para a janela imediata quando não há.

## Testes estatísticos clássicos

O caminho óbvio para decidir se duas amostras diferem é um teste de hipótese. Testes que supõem distribuição normal são tentadores, mas durações de span não são normais: têm cauda pesada, assimetria e frequentemente várias modas, por exemplo caminho com cache e caminho sem cache. Aplicar teste de médias nesses dados dá resultado frágil, e a média esconde justamente o que o SRE quer ver.

Testes baseados em postos, que comparam se um grupo tende a ter valores maiores que o outro, são mais robustos a essa forma. Testes que comparam a distribuição inteira, sensíveis a qualquer diferença de formato, também entram na lista. Eles detectam mudança na cauda sem que o centro se mexa, mas dizem pouco sobre qual aspecto mudou, o que atrapalha a explicação.

Um ponto comum a todos: com muita amostra, qualquer diferença minúscula fica estatisticamente significativa sem ter relevância operacional. Por isso o teste sozinho não basta; é preciso também um tamanho de efeito mínimo que importe para a operação. Definir esse mínimo é decisão de produto, e não está nesta nota.

## Bootstrap e intervalos de confiança

Outra família reamostra os dados para estimar a incerteza de uma estatística escolhida, por exemplo a diferença entre um percentil depois e antes. A vantagem é que se pode escolher a estatística que importa, como percentis altos, sem depender de fórmula fechada. O resultado vem como um intervalo, e o intervalo é fácil de mostrar ao SRE: a diferença provável está entre tal e tal.

Desvantagens: custo computacional maior, que precisa caber no orçamento do componente; e dependência de amostras razoavelmente independentes. Spans de um mesmo trace são correlacionados, e requisições em rajada também. Tratar tudo como independente subestima a incerteza. Reamostrar por trace, e não por span, mitiga isso em parte.

Percentis muito altos são instáveis com pouca amostra, e o bootstrap não conserta isso, só deixa o intervalo largo, o que é honesto. Pode-se rebaixar o percentil analisado quando o volume é baixo, deixando explícito no resultado que a medida mudou.

## Detecção de mudança em série temporal

Em vez de comparar duas janelas, trata-se a latência como série temporal e procura-se um ponto de mudança. Existem métodos acumulativos, que somam desvios pequenos e persistentes até cruzar um limite, e métodos que procuram a divisão da série que melhor separa dois regimes. Ambos têm a virtude de não exigir que se defina antes onde termina o antes e começa o depois; o deploy pode ser usado como hipótese a verificar, e não como entrada obrigatória.

Isso ajuda quando o deploy é gradual ou quando a regressão aparece com atraso, por exemplo depois que um cache esfria ou um dado cresce. Também pode detectar regressão que não coincide com deploy nenhum, o que amplia o escopo para algo parecido com monitoramento contínuo.

O preço é mais parâmetros para ajustar e menos clareza na explicação. Também há risco de apontar mudanças reais que não têm relação com o deploy, e o SRE ficar investigando a causa errada. Seria preciso cruzar o ponto detectado com os eventos de deploy antes de mostrar qualquer coisa.

## Modelos sazonais e previsão

Uma opção mais pesada é ajustar um modelo que prevê a latência esperada, incluindo ciclos diários e semanais, e marcar como regressão o que sai da faixa prevista. Há variações simples, como decomposição em tendência, ciclo e resíduo, e variações com aprendizado de máquina. O atrativo é tratar sazonalidade e tendência de forma sistemática, sem escolher manualmente a janela de comparação.

Os riscos são conhecidos. Modelos precisam de histórico, treino e acompanhamento, e quebram quando o serviço muda de regime. Quando erram, é difícil dizer por quê, o que vai contra o requisito de explicabilidade. Para uma equipe pequena, manter um conjunto de modelos por serviço e operação é trabalho contínuo. Fica como opção de segunda fase, se as abordagens por comparação de janelas se mostrarem insuficientes.

## Agregar no ClickHouse ou no processo Java

Independente do método, há a questão de onde calcular. O ClickHouse agrega muito bem: quantis aproximados, contagens por grupo e histogramas saem rápidos, e os dados não precisam trafegar. Se o detector só precisar de resumos por grupo, vale empurrar o máximo possível para a consulta e deixar o processo Java com a decisão final.

Testes que precisam das amostras individuais, como os baseados em postos ou o bootstrap, pedem trazer os dados para o processo, ou aproximá-los com resumos compactos. Trazer tudo cria pressão de memória e de rede; trazer uma amostra limitada cria viés se a amostragem não for bem feita. Uma via intermediária é pré-agregar em histogramas finos dentro do banco e fazer o teste sobre eles, aceitando a perda de resolução.

Também entra aqui a possibilidade de materializar agregados por janela, atualizados de forma incremental, de modo que a consulta do detector seja barata e repetível. Isso reduz a carga, mas fixa o formato dos agregados cedo, e mudar depois exige recalcular histórico.

## Granularidade da análise

Pode-se detectar regressão por serviço inteiro, por operação ou por combinação de operação com atributos, como rota, tipo de cliente ou região. Quanto mais fina, mais específico o aviso, mas menos amostra por grupo e mais comparações simultâneas. Muitas comparações ao mesmo tempo geram falsos positivos só por acaso, então é necessário algum controle de múltiplas comparações ou uma hierarquia: olhar primeiro o serviço, e só descer para operações quando o nível de cima indica algo.

Atributos de alta cardinalidade são um perigo particular. Agrupar por identificadores de usuário ou de requisição explode o número de grupos e não produz nada útil. Convém limitar os atributos de agrupamento a um conjunto pequeno e conhecido.

Outra escolha é olhar a latência do span da própria operação ou a latência exclusiva, descontando filhos. A exclusiva aponta melhor onde está a lentidão, porque separa o que a operação faz do que ela espera de dependências. Em compensação é mais cara de calcular e depende de a árvore de spans estar completa.

## Ruído, tráfego baixo e caudas

Serviços com pouco tráfego são o caso difícil. Poucas amostras significam que qualquer método fica ou cego ou barulhento. Opções: exigir um mínimo de amostras antes de emitir qualquer veredito e declarar "sem dados suficientes" caso contrário; ampliar a janela até haver amostra; ou juntar operações parecidas. Declarar a falta de dados é preferível a um alarme sem base.

As caudas merecem tratamento próprio. Uma regressão que afeta uma fração pequena das requisições mexe pouco no centro e muito nos percentis altos. Acompanhar só a mediana perde esse caso; acompanhar só o extremo gera ruído. Uma ideia é olhar mais de um ponto da distribuição e exigir coerência ou ao menos mostrar todos.

Há também outliers de natureza operacional: partida a frio de instâncias novas logo após o deploy, aquecimento de cache, compilação em tempo de execução na JVM. Esses efeitos são transitórios e inflam a latência no começo da janela pós-deploy sem representar regressão persistente. Descartar um período inicial de aquecimento ou comparar só depois que as instâncias estabilizam reduz alarmes falsos, mas esconde problemas reais de partida lenta, que também importam. Vale marcar esse período em vez de apagá-lo.

## Ligação com o deploy e com o Kubernetes

Para atribuir uma regressão a um deploy é preciso saber quando ele começou, quando terminou e quais instâncias rodam qual versão. Essas informações podem vir de atributos de recurso nos próprios spans, como a versão do serviço, ou de eventos do cluster. Usar o atributo de versão nos spans é mais direto e permite comparar versões lado a lado no mesmo intervalo, o que elimina o problema do ciclo de carga: as duas versões enfrentam a mesma carga ao mesmo tempo.

Essa comparação simultânea é atraente em estratégias de liberação gradual, como canário. Com parte do tráfego na versão nova, tem-se um grupo de controle natural. Os cuidados: o tráfego do canário pode não ser representativo, por exemplo se for roteado por tipo de cliente, e o tamanho do grupo novo é pequeno no começo.

Se o atributo de versão não estiver presente de forma confiável, o detector cai para a comparação por tempo, usando eventos de rollout como marcadores. Ficam casos de borda: reinícios sem mudança de versão, mudanças de configuração que não geram nova versão, e deploys de várias dependências quase juntos, quando atribuir a causa a um só é arriscado.

## Saída para Grafana e para o SRE

O resultado precisa ser lido por pessoas. Uma opção é gravar os veredictos e as estatísticas de apoio em tabelas e deixar o Grafana desenhá-los, com anotações marcando os deploys sobre os gráficos de latência. Outra é gerar um resumo textual curto por deploy, com os serviços e operações afetados, a direção da mudança e o grau de confiança.

O que mostrar junto do veredito: as duas distribuições comparadas, a medida de diferença com sua incerteza, o número de amostras de cada lado e a referência usada. Sem isso o SRE não consegue julgar se confia. Também ajuda um link para traces de exemplo da versão nova que ilustrem a lentidão, porque é isso que leva à investigação.

Sobre ordenação: listar por impacto estimado e não por significância estatística evita que diferenças minúsculas em operações irrelevantes ocupem o topo. Estimar impacto exige alguma noção de volume da operação, o que o ClickHouse fornece com facilidade.

## Perguntas em aberto

Algumas dúvidas ficaram sem resposta e afetam a escolha. Qual é a tolerância real das equipes a alarme falso, comparada ao custo de perder uma regressão? Isso decide o rigor dos testes. Como a amostragem de traces está configurada hoje e se ela distorce as caudas? Isso decide se spans brutos são confiáveis. Quantos serviços têm tráfego suficiente para qualquer método estatístico funcionar? Isso decide se o caso de pouco volume é exceção ou regra.

Também falta saber como as equipes fazem deploy na prática: se predominam liberações graduais com controle simultâneo ou trocas diretas. A primeira favorece comparação por versão; a segunda obriga comparação temporal. E se o detector deve se limitar ao pós-deploy ou servir de monitor contínuo, o que muda a família de métodos mais adequada.

## Como avaliar as opções

Antes de escolher, a ideia é montar um conjunto de avaliação com casos conhecidos: regressões reais passadas, regressões injetadas artificialmente em dados históricos, e períodos tranquilos para medir alarme falso. Cada opção roda sobre o mesmo conjunto, e se compara sensibilidade, taxa de falso positivo, custo de execução e clareza da explicação.

A injeção sintética precisa variar a forma da regressão: deslocamento geral, aumento só na cauda, aparecimento de uma segunda moda, degradação gradual. Métodos diferentes falham em formas diferentes, e testar só um tipo favorece sempre o mesmo método.

É razoável esperar que o resultado seja uma combinação: comparação simples com janela anterior ou por versão como base, um teste robusto para filtrar ruído, um piso de relevância operacional e a declaração explícita de dados insuficientes. Métodos de série temporal e de previsão ficam como reforço se a base não bastar. Isso é só uma expectativa de trabalho, que a avaliação pode desmentir.
