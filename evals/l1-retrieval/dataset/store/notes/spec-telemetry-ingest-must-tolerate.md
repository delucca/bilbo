---
id: 01KQMP899QY3SXPKK1276CM3SS
created: 2026-05-02T12:52-03:00
---

# Especificação do telemetry-ingest: tolerância a queda do broker MQTT

Esta nota especifica o que o `telemetry-ingest` precisa fazer quando o broker MQTT some. O requisito central é um só: o `telemetry-ingest` deve tolerar uma desconexão do broker MQTT de até `120 seconds` sem perder leituras. Tudo abaixo decorre disso. O resto é detalhe de como chegar lá sem inventar mecanismo novo no projeto. Quem lê só esta nota deve conseguir responder: quanto tempo de queda o componente aguenta, o que conta como perda, onde o dado fica retido e o que vem depois da reconexão.

O GridHaven prevê a produção solar residencial e agenda a carga da bateria contra tarifas por horário de uso. Se faltar leitura no meio do dia, a previsão fica pior e o agendamento de carga erra a janela de tarifa barata. Por isso a perda de leitura é tratada como defeito, não como degradação aceitável.

## Contexto e motivação

O `telemetry-ingest` fica entre os dispositivos das casas e o armazenamento de séries temporais. Os dispositivos publicam leituras de geração, consumo e estado da bateria. O componente assina, valida, normaliza e grava no InfluxDB. A previsão em Julia e a interface em Svelte leem dali, nunca direto do broker.

Redes domésticas são ruins. Roteador reinicia, o provedor oscila, o broker passa por manutenção ou falha de rede entre a nuvem e o Azure IoT Hub. Em campo isso acontece o tempo todo, e quase sempre dura pouco. A decisão foi cobrir as quedas curtas por completo e deixar as longas para um tratamento explícito, em vez de tentar garantir tudo para sempre.

Instaladores usam os painéis para diagnosticar instalações novas. Um buraco nos dados parece falha do equipamento e gera chamado desnecessário. Esse é o custo prático que a tolerância evita.

## Requisito principal

O requisito, escrito de forma direta: se a conexão entre o `telemetry-ingest` e o broker MQTT cair por até `120 seconds`, nenhuma leitura publicada nesse intervalo pode ser perdida. Ao reconectar, o componente deve entregar ao armazenamento todas as leituras que existiam, na ordem e com os carimbos de tempo originais.

Dois pontos de leitura do requisito. Primeiro, o limite vale para uma desconexão contínua. Várias quedas curtas seguidas contam cada uma por si, desde que o componente tenha recuperado o estado entre elas. Segundo, "sem perder" vale a partir do momento em que o dispositivo publicou com a garantia de entrega escolhida; leitura que nunca saiu do dispositivo está fora do escopo desta nota.

Acima de `120 seconds` o comportamento é outro, descrito mais adiante. Não há promessa de perda zero nesse caso, mas também não pode haver perda silenciosa.

## O que conta como perda

Perda é qualquer leitura que o dispositivo publicou com sucesso do ponto de vista dele e que nunca chegou ao InfluxDB. Também conta como perda a leitura que chegou com carimbo de tempo trocado pelo momento da reconexão, porque isso corrompe a série para a previsão. Esse segundo caso é o mais traiçoeiro, porque o dado "existe" mas está no lugar errado.

Duplicata não é perda, mas é defeito menor. A garantia de entrega pelo menos uma vez permite repetição, então a gravação precisa ser idempotente. Ver a seção de deduplicação.

Atraso não é perda. Uma leitura que chega minutos depois, com o carimbo correto, é aceitável, desde que a previsão consiga incorporar dado atrasado. Isso foi conversado e fica como premissa: o consumidor da série trata dado tardio como normal.

## Escopo

Está dentro do escopo: a assinatura no broker, o buffer local, a reconexão, a gravação no InfluxDB e a observabilidade desses passos. Está fora: a lógica de publicação nos dispositivos, a política interna de retenção do broker gerenciado e a previsão em Julia.

Também fora: qualquer garantia sobre dispositivos que ficam offline por conta própria. Eles podem guardar leituras e reenviar, mas isso é problema do firmware e de outra especificação. O `telemetry-ingest` só precisa aceitar o dado atrasado quando ele chegar, sem rejeitar por estar velho demais dentro de uma janela razoável.

## Premissas sobre o broker e o transporte

O broker MQTT fica atrás do Azure IoT Hub, ou ao lado dele, conforme o ambiente. O componente não deve depender de detalhe interno do broker para funcionar. A premissa é só esta: o protocolo oferece sessão persistente e níveis de qualidade de serviço, e o `telemetry-ingest` usa esses recursos em vez de reinventar.

Premissa de relógio: os dispositivos carimbam a leitura na origem. O componente confia no carimbo do dispositivo e só usa o relógio local como reserva quando o carimbo está ausente ou claramente inválido. Nesse caso a leitura é marcada como tendo carimbo estimado, para ninguém tratar como exata.

Premissa de ordem: dentro de um mesmo dispositivo a ordem de publicação é a ordem de leitura. Entre dispositivos não há ordem garantida e ninguém deve depender disso.

## Estratégia de sessão e entrega

O componente se conecta com sessão persistente, para o broker reter as mensagens destinadas à assinatura enquanto o cliente está fora. A assinatura usa qualidade de serviço que exige confirmação, ou seja, entrega pelo menos uma vez. Qualidade de serviço sem confirmação não serve aqui, porque perde mensagem em silêncio na queda.

A confirmação ao broker só é enviada depois que a leitura foi aceita pelo buffer durável ou gravada. Confirmar antes cria uma janela em que um travamento do processo perde dado já reconhecido. Essa ordem é regra, não otimização.

O identificador do cliente precisa ser estável entre reconexões. Se mudar a cada início, o broker trata como cliente novo e descarta o estado da sessão, e a tolerância some sem aviso. Isso já é o tipo de coisa que quebra em produção por mudança de configuração aparentemente inocente.

## Buffer local

Para cobrir a janela, o `telemetry-ingest` mantém um buffer local de leituras já recebidas e ainda não gravadas. Ele existe por dois motivos: absorver falha momentânea do InfluxDB e dar folga na reconexão, quando o volume atrasado chega de uma vez.

O buffer precisa sobreviver a reinício do processo. Memória pura não basta, porque uma queda do broker pode coincidir com redeploy. A escolha é uma fila em disco, simples, com gravação em ordem de chegada e remoção só após confirmação de escrita no InfluxDB.

O tamanho deve ser dimensionado para a taxa de leituras de pico vezes a janela de `120 seconds`, com folga. Não fixar o valor aqui; ele depende do número de dispositivos ativos e deve vir de configuração, com o cálculo documentado ao lado.

## Detecção de desconexão

Detectar rápido importa, porque o tempo de queda já consome parte da janela. O componente usa o mecanismo de keep-alive do protocolo e trata a falta de resposta como desconexão. O intervalo de keep-alive deve ser bem menor que a janela tolerada, para que a detecção não coma o orçamento de tempo.

Ao detectar, o componente registra o início da queda em log estruturado e muda o estado interno para desconectado. Esse estado é exposto como métrica. A partir daí começa a contagem do tempo de queda, usada para decidir se ainda estamos dentro da tolerância.

Cuidado com falso positivo: uma conexão lenta não é queda. Evitar derrubar e recriar a sessão por atraso pontual, porque recriar a sessão sem necessidade aumenta o risco de perder o estado retido.

## Reconexão

A reconexão usa recuo exponencial com limite superior e um pouco de aleatoriedade, para que muitos instâncias não batam no broker ao mesmo tempo quando ele volta. O limite superior do recuo precisa ser pequeno em relação à janela, senão uma tentativa que falha empurra a reconexão para fora da tolerância.

Ao reconectar, o componente retoma a sessão persistente e recebe o que o broker reteve. Em seguida drena o buffer local em ordem. Só depois de drenar passa ao estado normal de operação. Durante a drenagem, leituras novas entram na mesma fila, para não furar a ordem.

Se a sessão não foi retomada (o broker diz que começou do zero), o componente registra isso como evento de gravidade alta, porque indica que o estado retido pode ter se perdido e a garantia foi quebrada mesmo dentro do tempo.

## Gravação no InfluxDB

A gravação é em lote, com o carimbo original de cada leitura. O lote tem tamanho limitado para que uma recuperação com muito volume atrasado não estoure memória nem tempo de requisição. Falha de gravação não descarta o lote: ele volta para a fila e tenta de novo com recuo.

Cada ponto é identificado pela combinação de dispositivo, medida e carimbo. Assim a regravação do mesmo ponto sobrescreve o valor idêntico em vez de criar um segundo. Essa propriedade é o que torna segura a entrega pelo menos uma vez.

Se o InfluxDB estiver fora ao mesmo tempo que o broker, o buffer em disco segura o dado, mas a tolerância garantida continua sendo a do broker. Falha dupla fora da janela cai no tratamento de excedente.

## Deduplicação e idempotência

Como a entrega é pelo menos uma vez, repetição é esperada logo após reconectar. O componente não tenta eliminar toda duplicata na entrada; ele confia na idempotência da gravação. Uma verificação leve em memória, para as leituras mais recentes por dispositivo, reduz trabalho repetido, mas não é a garantia. A garantia é a chave do ponto.

Isso importa para quem for mexer na gravação: qualquer mudança que acrescente um campo variável à identidade do ponto, como um contador de recebimento ou o horário de chegada, quebra a idempotência e cria duplicatas reais na série. Não fazer.

Quando duas leituras com a mesma identidade chegam com valores diferentes, vale a mais recente em ordem de chegada, e o evento é contado numa métrica de conflito, porque indica problema no dispositivo.

## Quedas acima do limite

Se a desconexão passar de `120 seconds`, a promessa de perda zero deixa de valer. O que vale então: o componente continua tentando reconectar, não encerra, e ao voltar processa tudo que o broker ainda retiver. Parte das leituras pode ter sido descartada pelo broker por limite de retenção, e isso não é recuperável por este componente.

O requisito é que essa situação seja visível. O componente registra que a janela foi excedida, marca o intervalo afetado e emite alerta. Nunca pode parecer que está tudo normal quando há buraco. A série ganha o buraco real, sem preenchimento inventado; interpolar é decisão de quem consome, não de quem ingere.

Um buraco conhecido e anunciado é aceitável. Um buraco que ninguém percebe não é.

## Observabilidade

Métricas mínimas: estado da conexão, duração da queda atual, profundidade do buffer, idade da leitura mais antiga ainda não gravada, contagem de lotes que falharam, contagem de sessões não retomadas e contagem de conflitos de identidade. Todas por instância.

Alertas: sessão não retomada, queda se aproximando do limite de tolerância, queda que excedeu o limite, e buffer crescendo sem drenar. O alerta de aproximação deve disparar com margem, para dar tempo a alguém agir antes de estourar a janela.

Os logs são estruturados e trazem identificador do dispositivo quando fizer sentido. Não registrar o conteúdo completo das leituras em nível normal; é dado doméstico e não precisa ir para o log.

## Como testar

O teste principal é de queda controlada: derrubar a conexão com o broker por um tempo dentro da tolerância, enquanto um gerador publica leituras com carimbos conhecidos, e comparar no InfluxDB o que foi publicado com o que foi gravado. O resultado esperado é conjunto idêntico, sem faltas e sem carimbos trocados.

Variações obrigatórias: queda exatamente no limite; queda com reinício do processo no meio; queda com o InfluxDB lento; queda repetida em sequência; e queda acima do limite, verificando que o alerta sai e que o componente se recupera sozinho. Cada variação deve ser automatizada e rodar no pipeline, não só na mão.

Teste de idempotência separado: reenviar o mesmo conjunto duas vezes e conferir que a série não muda.

## Configuração

A tolerância é um requisito do produto, então fica explícita na configuração e não enterrada em constante de código. Um exemplo curto do valor que governa o dimensionamento:

```
broker_disconnect_tolerance = 120 seconds
```

Os demais parâmetros (keep-alive, limites do recuo, tamanho de lote, tamanho do buffer) são derivados desse valor e da taxa de pico. Quem alterar a tolerância precisa revisar todos eles juntos, porque o buffer e o recuo foram pensados em função dela. Mudar só um número deixa a garantia falsa.

## Riscos e pontos em aberto

Risco principal: a retenção do broker gerenciado pode ter limites próprios que não conhecemos bem para volume alto. Precisa ser confirmado com o ambiente real antes de se afirmar a garantia para toda a base de clientes.

Segundo risco: disco cheio no buffer. Se o disco encher, o componente precisa recusar a confirmação ao broker em vez de confirmar e descartar. Falta decidir o comportamento exato de proteção e se haverá alerta antecipado de espaço.

Em aberto: se vale elevar a tolerância no futuro para cobrir manutenções planejadas do broker mais longas. Se sim, o custo é buffer maior e mais tempo de drenagem, e a previsão fica mais tempo com dado atrasado. Não há decisão ainda; o requisito vigente é o descrito no começo desta nota.

## Resumo para quem tem pressa

O `telemetry-ingest` aguenta queda do broker de até `120 seconds` sem perder leitura. Para isso usa sessão persistente, identificador de cliente estável, confirmação só depois de segurar o dado, fila em disco e gravação idempotente no InfluxDB com o carimbo original. Acima do limite não há promessa de perda zero, mas a falha tem que aparecer em log, métrica e alerta. Se mexer em qualquer uma dessas peças, reexecute os testes de queda controlada antes de aceitar a mudança.
