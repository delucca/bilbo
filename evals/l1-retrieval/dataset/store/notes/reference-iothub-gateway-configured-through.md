---
id: 01KRJSBP4WH53HM887NP8ST2SR
created: 2026-05-14T05:23-03:00
sources:
  - "doc: iothub-gateway runbook"
---

# iothub-gateway: referência de configuração e operação

O `iothub-gateway` é o componente do GridHaven que fica entre o Azure IoT Hub e o resto do sistema. Ele recebe do hub as leituras enviadas pelos inversores, medidores e baterias instalados nas casas dos clientes. Depois entrega essas leituras para quem precisa delas: o armazenamento de séries temporais em InfluxDB e os serviços em Julia que fazem a previsão de geração solar e o agendamento de carga da bateria contra a tarifa horária. Esta nota é uma referência rápida para quem vai mexer no componente ou investigar um problema nele. Foi escrita com pressa, então vale conferir no código quando um detalhe importar.

## Nome do componente

O nome atual é `iothub-gateway`. O componente se chamava `hubrelay` antes. Houve a troca de nome, e hoje o certo é `iothub-gateway` em qualquer lugar novo: documentação, painéis, alertas, mensagens de commit, conversas com instaladores.

A troca de nome é a principal armadilha de busca. Quem procurar histórico, tíquetes antigos, mensagens de chat ou postagens de incidentes só por `iothub-gateway` não vai achar nada do período anterior. Procure também por `hubrelay`. O contrário vale igual: se aparecer `hubrelay` num painel, num runbook antigo, num nome de alerta ou num comentário de código, é o mesmo componente que hoje se chama `iothub-gateway`. Não é um serviço separado nem um que tenha sobrado. Ao encontrar essas referências antigas, a atitude certa é atualizar o texto para o nome novo quando o custo for baixo. Quando não for, deixe uma frase dizendo que `hubrelay` é o nome anterior.

Para quem chega agora no projeto, a regra prática é simples. Se alguém falar em `hubrelay`, entenda `iothub-gateway`. Ao escrever, use só `iothub-gateway`, a não ser que esteja citando de propósito algo antigo, como o título de um tíquete ou o nome de um arquivo que ainda não foi renomeado.

## Configuração por variáveis de ambiente

O `iothub-gateway` é configurado por variáveis de ambiente. As duas que importam para a ligação com o Azure IoT Hub são `IOTHUB_CONNECTION_STRING` e `IOTHUB_CONSUMER_GROUP`.

`IOTHUB_CONNECTION_STRING` é a cadeia de conexão com o Azure IoT Hub. É por ela que o componente sabe a qual hub se conectar e com qual credencial. Ela é segredo. Não vai para o repositório, não vai para logs, não vai colada em tíquete e não deve aparecer em saída de diagnóstico. Em ambientes reais ela vem do mecanismo de segredos da plataforma onde o serviço roda. Em desenvolvimento local, cada pessoa usa a sua própria, apontando para um hub de testes, nunca para o hub de produção.

`IOTHUB_CONSUMER_GROUP` diz qual grupo de consumidores do hub o componente usa para ler os eventos. Na prática, o grupo de consumidores é o que mantém a leitura do `iothub-gateway` independente de outros leitores do mesmo hub. Se dois processos diferentes lerem com o mesmo grupo, eles passam a disputar as mesmas partições e cada um enxerga só uma parte do fluxo, ou os dois se atrapalham. Por isso o valor de `IOTHUB_CONSUMER_GROUP` deve ser dedicado ao `iothub-gateway`, em cada ambiente. Ferramentas de inspeção e depuração que lerem do mesmo hub devem usar outro grupo.

As duas variáveis são necessárias para o componente subir e consumir. Sem `IOTHUB_CONNECTION_STRING` ele não tem como falar com o hub. Sem `IOTHUB_CONSUMER_GROUP` ele não sabe por onde ler, e o comportamento depende de como a configuração trata o valor ausente. Não conte com um padrão silencioso: defina as duas explicitamente em todo ambiente.

Na dúvida sobre se um ambiente está configurado certo, confira primeiro essas duas variáveis, antes de procurar o problema em outro lugar. Boa parte dos casos de "o gateway não recebe nada" vem de uma cadeia de conexão vencida ou trocada, ou de um grupo de consumidores compartilhado por engano.

## Papel no fluxo de dados

O caminho geral dos dados é este. O dispositivo na casa do cliente mede produção solar, consumo e estado da bateria. Ele publica essas medições, e o Azure IoT Hub as recebe. Alguns dispositivos falam MQTT direto com o hub. Outros passam antes por um concentrador local que traduz o que o equipamento fala para MQTT. O `iothub-gateway` lê os eventos do hub usando o grupo de consumidores configurado e os normaliza. Em seguida grava as leituras no InfluxDB e as deixa disponíveis para os serviços de previsão e de agendamento, escritos em Julia. A interface Svelte usada por instaladores e clientes lê os dados já processados. Ela não fala com o hub nem com o `iothub-gateway`.

A normalização é uma parte importante do trabalho do componente. Equipamentos de fabricantes diferentes mandam unidades, nomes de campo e frequências de envio diferentes. O `iothub-gateway` converte tudo para o formato interno do GridHaven antes de gravar. Quando um equipamento novo entra em campo, o ajuste costuma ser feito aqui, e não nos serviços que consomem os dados. A ideia é que o restante do sistema nunca precise saber de qual fabricante veio uma medição.

O componente também cuida dos sentidos de volta quando existem. Comandos do agendador de bateria, como carregar agora ou segurar a carga até o fim da tarifa cara, saem do sistema e precisam chegar ao dispositivo. O caminho passa pelo hub. O `iothub-gateway` é o ponto por onde esse tráfego sai. Isso quer dizer que uma falha de conexão com o hub afeta os dois sentidos: o sistema deixa de receber leituras e deixa de conseguir mandar ordens.

O que o `iothub-gateway` não faz: ele não prevê nada, não decide quando carregar a bateria e não guarda histórico longo por conta própria. Previsão e agendamento são dos serviços em Julia. O histórico fica no InfluxDB. Se aparecer vontade de pôr regra de negócio no gateway, é sinal de que ela pertence a outro lugar.

## Operação e diagnóstico

Quando algo parece errado, vale seguir uma ordem fixa, da ponta de entrada para a de saída.

Primeiro, confirme que o processo está de pé e que as variáveis `IOTHUB_CONNECTION_STRING` e `IOTHUB_CONSUMER_GROUP` estão definidas no ambiente em que ele roda. Muita gente perde tempo olhando dados quando o serviço nem subiu direito, ou subiu com a configuração de outro ambiente.

Segundo, veja se o hub está recebendo mensagens dos dispositivos. Se o hub não recebe, o problema está antes do `iothub-gateway`: rede da casa, equipamento, concentrador local ou credencial do dispositivo. Nesse caso mexer no gateway não ajuda. O painel do próprio Azure IoT Hub mostra se há tráfego chegando, e é a forma mais rápida de separar "não chega ao hub" de "chega ao hub e não passa do gateway".

Terceiro, se o hub recebe e o gateway não entrega, olhe a conexão do gateway com o hub. As causas mais comuns são credencial vencida na cadeia de conexão, grupo de consumidores errado ou compartilhado e falha de rede entre o gateway e o Azure. Os logs do componente dizem qual delas é. Evite colar linhas de log inteiras em lugares públicos, pois podem trazer trechos de configuração sensíveis.

Quarto, se o gateway lê mas os dados não aparecem do outro lado, olhe a gravação no InfluxDB. Aqui os suspeitos são indisponibilidade do banco, credencial de escrita e rejeição de pontos por formato. Uma leitura mal normalizada pode ser descartada pelo banco, e o sintoma é um buraco no gráfico de um único dispositivo enquanto os outros seguem normais. Buraco isolado em um dispositivo costuma ser problema de normalização daquele modelo de equipamento. Buraco em todos ao mesmo tempo costuma ser problema de conexão ou de banco.

Um sintoma que engana: o previsor de geração solar começa a dar resultados estranhos, ou o agendador carrega a bateria na hora errada. À primeira vista parece bug nos serviços em Julia. Com frequência a causa é dado atrasado ou faltando que vem do gateway. Antes de mexer no modelo, confira se as leituras mais recentes realmente estão chegando e com horário certo.

## Pontos de atenção

Primeiro, o nome. Os dois nomes, `hubrelay` e `iothub-gateway`, aparecem espalhados em documentos, painéis e histórico. Trate como a mesma coisa. Ao procurar algo antigo, busque pelos dois. Ao escrever algo novo, use `iothub-gateway`.

Segundo, o grupo de consumidores. Reaproveitar o valor de `IOTHUB_CONSUMER_GROUP` em uma ferramenta de depuração é um erro fácil de cometer e difícil de notar. O sintoma é o gateway perder parte das mensagens sem erro claro. Quem precisar espiar o fluxo de um hub deve usar um grupo próprio para isso.

Terceiro, o segredo. `IOTHUB_CONNECTION_STRING` dá acesso ao hub. Se vazar, a cadeia deve ser trocada no Azure, e o componente reiniciado com o valor novo. Isso vale para qualquer ambiente, inclusive os de teste, porque um hub de teste mal protegido ainda é uma porta aberta.

Quarto, ambientes. Cada ambiente deve ter o seu hub, a sua cadeia de conexão e o seu grupo de consumidores. Apontar um ambiente local para o hub de produção, mesmo que só para olhar, mistura dados de clientes reais com testes e pode disparar comandos reais em baterias reais.

Quinto, dependências em cadeia. Como o gateway é o ponto de entrada dos dados de campo e o ponto de saída dos comandos, uma parada dele tem efeito amplo. As previsões ficam sem dados novos, o agendamento passa a trabalhar com informação velha e a interface mostra números parados. Os serviços a jusante precisam tolerar dados atrasados sem tomar decisões ruins, mas isso não substitui avisar rápido quando o gateway cai. Alertas sobre o `iothub-gateway` devem chegar a quem pode agir, e o nome do alerta deve usar o nome atual.

## O que ainda não está registrado aqui

Esta nota cobre apenas o que está confirmado: o nome atual e o anterior, as duas variáveis de ambiente de ligação com o hub e o papel geral do componente no fluxo. Não há aqui valores de configuração, nomes de tópicos, formatos exatos de mensagem, limites de taxa nem detalhes de implantação. Se algum desses pontos for necessário, procure no código e na configuração do ambiente em questão, e acrescente à nota o que for confirmado, em vez de confiar em memória.

Se você mudar o comportamento de configuração do componente, por exemplo adicionando outra variável de ambiente ou alterando o tratamento de um valor ausente, atualize esta nota no mesmo momento. A parte de configuração é a que mais gera dúvida de quem chega, e uma referência desatualizada nessa parte custa mais do que nenhuma.
