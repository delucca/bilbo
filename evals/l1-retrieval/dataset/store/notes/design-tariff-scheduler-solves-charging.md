---
id: 01M02C957JHCHEHR8P1C1ZPMGM
created: 2026-08-15T06:33-03:00
---

# Design do tariff-scheduler: agendamento de carga da bateria como programa linear

Esta nota registra como o `tariff-scheduler` decide quando carregar e descarregar a bateria de uma casa no GridHaven. O GridHaven prevê a produção solar doméstica e agenda a carga da bateria contra tarifas por horário de uso. O `tariff-scheduler` é a parte que transforma previsão e tarifa em um plano de carga. Escrevi com pressa, para o próximo engenheiro ou agente não precisar redescobrir o desenho. Se algo aqui divergir do código, confie no código e corrija a nota.

O ponto central: o `tariff-scheduler` resolve a carga como um programa linear, usando `JuMP` para modelar e o solver `HiGHS` para resolver, sobre `96 quarter-hour slots` por dia. Ou seja, o dia é cortado em intervalos de quinze minutos, e para cada intervalo o programa decide quanta energia entra ou sai da bateria. Tudo o que está abaixo é consequência dessa escolha ou ajuda a entender por que ela foi feita.

Nome antigo: o componente se chamava `chargeplan`. Hoje o nome é `tariff-scheduler`. Se você encontrar `chargeplan` em issues antigas, em dashboards, em mensagens de commit, em tópicos de mensagens ou em conversas com instaladores, é a mesma coisa. Não existe um segundo componente. Ao escrever documentação nova, use sempre o nome atual. Ao buscar histórico, pesquise pelos dois nomes, porque muita decisão antiga está registrada sob o nome velho.

## Contexto e problema

O cliente final tem painéis solares, uma bateria e uma tarifa que muda ao longo do dia. Em geral há um período barato, um intermediário e um caro, e a diferença entre eles é o que torna a bateria rentável. O instalador configura o sistema e acompanha vários clientes. O cliente quer que a conta de luz caia sem ter que pensar nisso. O trabalho do `tariff-scheduler` é decidir, com antecedência, em que momentos comprar energia da rede para encher a bateria, em que momentos deixar a bateria alimentar a casa e em que momentos guardar a sobra solar em vez de exportá-la a preço baixo.

Sem agendamento, a bateria costuma seguir uma regra simples: carrega com o sol, descarrega quando a casa consome mais do que o painel produz. Isso é razoável, mas ignora a tarifa. Num dia nublado, a regra simples esvazia a bateria no fim da tarde, justamente quando a energia da rede está mais cara, e deixa a noite inteira sem reserva. O agendador olha para a frente: se a previsão solar do dia seguinte é ruim e a madrugada é barata, vale carregar da rede de madrugada, mesmo que a bateria já tenha alguma carga. Se a previsão é boa, vale deixar espaço para o sol.

Esse tipo de decisão depende de três coisas ao mesmo tempo: a previsão de produção, a previsão de consumo da casa e a tabela de tarifas. O agendador recebe as três como séries alinhadas no mesmo tempo e devolve um plano de carga e descarga alinhado com elas. A previsão solar em si não é responsabilidade deste componente; ela vem do módulo de previsão e chega já pronta. Aqui só consumimos o resultado.

Outra restrição de contexto: o sistema roda em hardware doméstico e na nuvem, e os instaladores esperam que o plano seja recalculado com frequência razoável sem exigir máquina grande. Isso pesou na escolha do método. Um problema que resolve em pouco tempo, de forma previsível e sem depender de sorte, é muito mais fácil de operar do que uma busca heurística cujo resultado varia entre execuções.

## Por que programação linear

A decisão de usar um programa linear veio de três razões práticas. A primeira é que o problema é naturalmente linear na maior parte do que importa. O custo de um intervalo é o preço da tarifa vezes a energia comprada da rede, menos a receita pela energia exportada. O estado de carga da bateria evolui somando entradas e subtraindo saídas, com perdas modeladas como fatores multiplicativos. Os limites de potência e de capacidade são desigualdades simples. Nada disso exige não linearidade para dar um plano útil.

A segunda razão é a garantia. Um programa linear tem ótimo global, e o solver diz quando o encontrou ou quando o problema é inviável. Não existe o caso de um plano ruim porque a busca ficou presa num mínimo local. Quando o plano parece estranho, a explicação está nos dados ou nas restrições, e isso é depurável. Num sistema que o instalador precisa defender diante do cliente, poder dizer por que a bateria carregou naquela hora é valioso.

A terceira razão é o ecossistema. Em Julia, o `JuMP` é a forma natural de escrever modelos de otimização: a sintaxe fica próxima da matemática, o que facilita revisão. O `HiGHS` é um solver de código aberto que resolve programas lineares com bom desempenho e se integra ao `JuMP` sem atrito. Não precisamos de licença comercial, e isso importa para um produto vendido a instaladores pequenos. Se um dia for preciso trocar o solver, o modelo escrito em `JuMP` continua o mesmo e só muda a linha que escolhe o otimizador.

Alternativas que foram consideradas e deixadas de lado: regras fixas por faixa de tarifa, que são simples mas desperdiçam o valor da previsão; programação dinâmica sobre o estado de carga, que lida bem com não linearidades mas cresce mal quando se somam restrições; e métodos heurísticos ou de aprendizado, que são difíceis de explicar e de testar. A programação linear ficou no meio-termo certo: expressiva o bastante, rápida e explicável.

Vale registrar a limitação que aceitamos. Por ser linear, o modelo não representa bem coisas como eficiência de conversão que varia com a potência, ou o custo de desgaste da bateria como função não linear da profundidade de descarga. Essas coisas são aproximadas por fatores constantes e por um custo linear de ciclagem. Se a aproximação começar a gerar planos ruins, a discussão certa é se ela basta ou se é preciso mudar de formulação, e não remendar o modelo atual com truques.

## Discretização em intervalos de quinze minutos

O dia é dividido em `96 quarter-hour slots`. A escolha de quinze minutos não é arbitrária: casa com a granularidade de várias tarifas e de vários medidores, e é fina o bastante para capturar mudanças de preço e picos de consumo sem tornar o problema grande demais. Com esse tamanho, o programa tem um número de variáveis e restrições pequeno para os padrões de um solver como o `HiGHS`, e a resolução é rápida.

Para cada intervalo, as variáveis principais são a energia comprada da rede, a energia exportada, a energia que entra na bateria, a energia que sai dela e o estado de carga ao final do intervalo. As restrições amarram essas variáveis: o balanço de energia da casa precisa fechar em cada intervalo, considerando produção solar prevista, consumo previsto, rede e bateria; o estado de carga segue da carga anterior mais entradas menos saídas, com as perdas; e tudo respeita os limites de potência e de capacidade configurados para aquele equipamento.

A função objetivo minimiza o custo total do dia: compras da rede ponderadas pela tarifa de cada intervalo, menos a receita de exportação, mais um termo pequeno de penalização de uso da bateria, para o plano não ciclar à toa por diferenças ínfimas de preço. Esse termo é o que evita planos que alternam carga e descarga em intervalos vizinhos sem ganho real. Ele é deliberadamente pequeno, e quem mexer nele precisa olhar planos reais antes e depois.

Um detalhe que costuma confundir: o horizonte do dia é fixo em intervalos, mas o início do plano nem sempre coincide com a meia-noite. Quando o plano é recalculado no meio do dia, os intervalos já passados deixam de ser variáveis e viram dados observados, e o estado de carga inicial vem da medição mais recente. O número de intervalos livres, portanto, diminui ao longo do dia. Para evitar que o fim do dia seja otimizado como se nada existisse depois, o modelo inclui um valor terminal para a energia que sobra na bateria, ligado à tarifa esperada do começo do dia seguinte.

A tabela de tarifas precisa ser expandida para a mesma grade. Tarifas por horário de uso normalmente são definidas em faixas largas, e a expansão repete o preço da faixa em cada intervalo que ela cobre. Mudanças de horário de verão e dias especiais, como feriados com tarifa diferente, são tratados na etapa que monta a tabela, antes do modelo. O modelo em si assume que recebe uma grade completa e coerente, e falha cedo se isso não acontecer.

## Entradas, saídas e integração

O `tariff-scheduler` não fala direto com a bateria nem com os painéis. Ele fica no meio de um fluxo de dados. As medições de campo chegam por MQTT, passam pelo Azure IoT Hub e são gravadas no InfluxDB como séries temporais. O agendador lê do InfluxDB o histórico recente, necessário para estimar o consumo e conhecer o estado atual da bateria, e recebe a previsão solar do módulo de previsão. As tarifas vêm da configuração de cada instalação, mantida pelo instalador.

A saída é um plano: para cada intervalo, a potência alvo de carga ou descarga da bateria, mais o custo esperado. O plano é gravado de volta no InfluxDB, para consulta e auditoria, e publicado para o equipamento por MQTT, via Azure IoT Hub, como comando de agenda. A interface em Svelte lê o mesmo plano e o mostra ao instalador e ao cliente, com o custo previsto e uma explicação curta de por que a bateria vai carregar ou descarregar em cada período.

O equipamento trata o plano como uma sugestão forte, não como ordem cega. Se a medição local divergir muito do previsto, por exemplo uma nuvem inesperada ou um pico de consumo, o controlador local tem regras de segurança que prevalecem, como nunca descarregar abaixo do mínimo configurado. O agendador, por sua vez, é recalculado periodicamente com dados novos, e é esse ciclo de recálculo que corrige as divergências do plano anterior. Não tentamos fazer o plano perfeito; tentamos fazê-lo bom e refeito com frequência.

Sobre a nomenclatura, lembre que os tópicos e nomes de séries mais antigos podem trazer o nome `chargeplan`. Em alguns lugares a renomeação foi feita e em outros ficou o rastro. Ao mexer em tópicos de mensagens, confirme primeiro quem ainda assina o nome antigo; trocar às cegas pode deixar equipamentos em campo sem receber plano. Se for preciso migrar, faça com período de convivência dos dois nomes, e só depois remova o antigo.

A integração com o Azure IoT Hub traz uma consequência de desenho: as mensagens podem chegar atrasadas, duplicadas ou fora de ordem. O agendador não assume entrega perfeita. Cada plano carrega a informação de quando foi calculado, e o equipamento descarta um plano mais antigo se já tiver um mais novo. Do lado da leitura, o agendador tolera lacunas na série do InfluxDB, preenchendo com a melhor estimativa disponível e sinalizando que o dado é estimado.

## Operação, falhas e testes

O caso de falha mais comum é um problema inviável ou mal condicionado, quase sempre causado por dado ruim: previsão com valor ausente, tarifa incompleta, estado de carga fora do limite físico. O comportamento esperado é registrar o motivo de forma clara e cair para um plano de reserva, simples e conservador, em vez de deixar o equipamento sem agenda. O plano de reserva segue regras fixas por faixa de tarifa, o que é pior que o ótimo mas seguro. Quando o plano de reserva é usado, isso fica marcado nos dados, para que o instalador veja e para que a gente possa medir com que frequência acontece.

Outro caso é a demora excessiva do solver. Com a grade de `96 quarter-hour slots`, o problema é pequeno e isso raramente ocorre, mas existe um limite de tempo configurado. Se estourar, usamos a melhor solução disponível quando houver, ou o plano de reserva. Se isso começar a acontecer com frequência, a causa provável não é o tamanho do problema e sim alguma restrição mal escrita ou números com escalas muito diferentes. Vale olhar a escala das unidades antes de culpar o `HiGHS`.

Para testar, usamos casos pequenos com resposta conhecida, onde dá para calcular o plano ótimo à mão: um dia com tarifa barata na madrugada e cara à noite, sem sol, deve carregar na madrugada e descarregar à noite até o limite da capacidade. Há também casos de propriedade, que verificam coisas que qualquer plano válido precisa cumprir: o balanço de energia fecha em cada intervalo, o estado de carga fica entre os limites, e a potência respeita o máximo. Esses testes pegam a maioria dos erros de modelagem. Também mantemos alguns dias reais anonimizados como regressão, para notar quando uma mudança altera planos de forma inesperada.

Ao mudar o modelo, o fluxo recomendado é comparar o custo previsto do plano novo com o do antigo sobre os mesmos dias de regressão. Uma queda pequena de custo, acompanhada de planos que fazem sentido, é boa notícia. Uma queda grande e suspeita normalmente indica que uma restrição foi afrouxada sem querer. Desconfie de planos que ficam bons demais.

## Decisões abertas e armadilhas

Algumas coisas ficaram em aberto de propósito. A primeira é a incerteza da previsão solar. Hoje o modelo usa uma previsão pontual e confia nela. Uma versão estocástica, com cenários, poderia proteger melhor contra dias em que a previsão erra muito, mas multiplicaria o tamanho do problema e complicaria a explicação para o cliente. Por ora, a estratégia é recalcular com frequência e deixar as regras de segurança do equipamento absorverem o erro. Se os dados de campo mostrarem perda relevante por causa disso, esta é a primeira coisa a rever.

A segunda é o desgaste da bateria. O custo linear de ciclagem é uma aproximação grosseira. Fabricantes diferentes têm curvas de degradação diferentes, e o valor certo para o termo de penalização depende do equipamento. Hoje ele é um parâmetro de configuração com um padrão conservador. Calibrar isso por modelo de bateria seria um bom trabalho futuro, mas exige dados de longo prazo que ainda não temos em volume suficiente.

A terceira é a exportação. Em alguns mercados a energia exportada é paga a um preço fixo baixo, em outros acompanha a tarifa, e em outros há limites de exportação impostos pela distribuidora. O modelo tem um ponto onde essas regras entram, mas nem todos os casos estão cobertos. Antes de prometer a um instalador que uma regra local é suportada, confira se ela cabe nas restrições existentes.

Armadilhas conhecidas, em ordem de frequência. Confundir unidades de potência e de energia: o intervalo é de quinze minutos, então converter potência em energia exige o fator de tempo correto, e esse erro gera planos que parecem quase certos e estão errados por um fator constante. Esquecer fuso e horário de verão ao alinhar a tarifa com a grade, o que desloca as faixas e faz a bateria carregar na hora errada. Tratar lacuna de dados como zero, o que faz o consumo parecer menor do que é. E procurar o componente pelo nome antigo `chargeplan` só em um lugar, deixando passar referências antigas em configurações e painéis.

Por fim, uma regra de bolso para quem for alterar este desenho: mantenha o modelo linear enquanto der. Cada vez que alguém propõe um refinamento, a pergunta útil é se ele cabe como uma restrição ou um termo linear. Se couber, entra no `JuMP` sem drama e o `HiGHS` continua resolvendo rápido. Se não couber, a conversa é outra e deve virar uma nota de decisão própria, com os dados que justificam sair do programa linear, em vez de um acréscimo silencioso aqui.

Se esta nota ficar desatualizada, atualize no lugar em vez de criar outra sobre o mesmo assunto. O assunto é o desenho do `tariff-scheduler`, antes chamado `chargeplan`, e este é o lugar dele.
