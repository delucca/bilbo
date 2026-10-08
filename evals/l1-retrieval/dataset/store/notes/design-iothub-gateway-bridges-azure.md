---
id: 01KRMT8Q6RG98HXC9S0KG386ZX
created: 2026-05-15T00:18-03:00
---

# Design do iothub-gateway

O `iothub-gateway` liga o Azure IoT Hub ao broker MQTT interno do GridHaven. Ele lê os eventos device-to-cloud pelo `Event Hubs-compatible endpoint` do IoT Hub e republica cada evento no broker interno. Quem consome do broker (a previsão de geração solar em Julia, o agendador de carga da bateria, a gravação no InfluxDB e o painel em Svelte) não precisa saber que existe um IoT Hub no meio. Esta nota registra o desenho, o motivo de cada escolha e os pontos que costumam dar problema.

Os inversores e medidores dos clientes falam com o Azure IoT Hub. O restante do sistema fala MQTT interno. O gateway fica entre os dois e só faz a ponte, sem regra de negócio. Se uma regra começar a aparecer nele, o lugar dela é outro serviço.

## Papel e limites do componente

O que o `iothub-gateway` faz:

- lê os eventos de telemetria que os dispositivos enviam ao IoT Hub, pelo `Event Hubs-compatible endpoint`;
- converte o envelope do evento para uma mensagem MQTT com tópico e payload no formato que o resto do sistema espera;
- publica a mensagem no broker interno;
- só confirma o avanço da leitura depois que a publicação foi aceita pelo broker.

O que ele não faz:

- não grava nada no InfluxDB. A gravação é um consumidor MQTT separado, e assim o gateway não depende do banco;
- não calcula previsão nem decide horário de carga. Isso é da parte em Julia;
- não envia comandos de volta aos dispositivos. O caminho cloud-to-device é outro assunto e não passa por aqui. Se um dia for preciso, abrir uma nota própria, porque o desenho muda bastante (confirmações, expiração, ordem);
- não guarda estado de negócio. O único estado dele é a posição de leitura em cada partição, e esse estado fica fora do processo.

O motivo para ler pelo `Event Hubs-compatible endpoint`, e não montar uma rota própria para outro destino, é que esse endpoint já vem com o IoT Hub, tem partições, mantém ordem dentro de cada partição e permite retomar a leitura de onde parou. Não precisamos de mais um serviço de fila só para isso. O custo é aceitar as regras de retenção e de grupos de consumidores desse endpoint, descritas abaixo.

## Fluxo dos dados

O caminho de um evento, do dispositivo até o consumidor:

1. O dispositivo do cliente (inversor, medidor ou controlador de bateria) envia telemetria ao Azure IoT Hub.
2. O IoT Hub deixa o evento disponível no `Event Hubs-compatible endpoint`, dentro de uma partição.
3. O `iothub-gateway` lê a partição, extrai o identificador do dispositivo e as propriedades de sistema que o IoT Hub anexa ao evento.
4. O gateway monta o tópico MQTT a partir do identificador do dispositivo e do tipo de mensagem, e publica o payload no broker interno.
5. Os consumidores internos recebem a mensagem. Um deles escreve no InfluxDB; outros alimentam a previsão e o agendamento.
6. Com o broker tendo aceitado a mensagem, o gateway avança a posição de leitura daquela partição.

A ordem de entrega vale por partição, e o IoT Hub distribui por dispositivo, então as mensagens de um mesmo dispositivo chegam em ordem. Entre dispositivos diferentes não há garantia nenhuma, e nenhum consumidor deve depender disso.

O gateway não reescreve o conteúdo medido. Valores de potência, energia e estado da bateria passam como vieram. A única adição é metadado: identificador do dispositivo, horário de enqueue do IoT Hub e, quando existir, o horário que o próprio dispositivo informou. Os dois horários ficam separados de propósito, porque dispositivos com relógio errado são comuns em instalações residenciais, e a previsão precisa saber qual horário está usando.

## Decisões de projeto e motivos

### Entrega pelo menos uma vez

O gateway só avança a posição depois da publicação aceita pelo broker. Se o processo cair entre a publicação e o avanço, o evento é lido de novo e publicado de novo. Escolhemos isso em vez de risco de perda: perder uma leitura de geração solar deixa um buraco na série e piora a previsão, enquanto uma leitura duplicada é inofensiva se o consumidor for idempotente. A gravação no InfluxDB já é idempotente para o mesmo ponto na mesma série com o mesmo horário, então o duplicado só sobrescreve com o mesmo valor. Quem criar um consumidor novo precisa tratar duplicata. Isso deve estar escrito na documentação do consumidor.

### Posição de leitura fora do processo

A posição por partição fica em armazenamento externo, não em memória nem em arquivo local. Assim, reiniciar ou trocar o contêiner não faz o gateway reler tudo nem pular eventos. Com mais de uma instância, o armazenamento externo também serve para dividir as partições entre elas, de modo que cada partição tenha um único leitor por vez. Duas instâncias lendo a mesma partição produzem duplicatas em volume e quebram a ordem, e isso já foi a primeira coisa a verificar quando apareceu mensagem fora de ordem.

### Grupo de consumidores próprio

O gateway usa um grupo de consumidores dedicado no `Event Hubs-compatible endpoint`. Não usar o grupo padrão, porque outras ferramentas (inspeção manual, scripts de diagnóstico) o usam e disputariam a leitura. Quem precisar olhar os eventos brutos para depurar deve criar um grupo separado e não reaproveitar o do gateway, senão a posição de leitura é afetada.

### Tópicos MQTT estáveis

O formato dos tópicos publicados é um contrato com os consumidores. Mudar o formato quebra a gravação e o agendador sem erro visível, já que simplesmente deixam de receber mensagens. Qualquer mudança nesse formato precisa de plano de migração, com publicação nos dois formatos por um período, e não deve entrar junto com outra alteração do gateway.

### Autenticação e segredos

A cadeia de conexão do endpoint e as credenciais do broker vêm do ambiente de execução, nunca do repositório. O gateway deve ter só a permissão de leitura de que precisa no IoT Hub, sem permissão de gerenciar dispositivos. Rotacionar a chave exige reiniciar o serviço; deixar isso previsto no procedimento de operação.

## Pontos de atenção e operação

**Retenção.** Os eventos no `Event Hubs-compatible endpoint` ficam disponíveis por um tempo limitado. Se o gateway ficar parado além desse tempo, os eventos mais antigos são perdidos e não há como recuperá-los por aqui. Por isso o alerta de gateway parado deve disparar bem antes do fim da retenção. Depois de uma parada longa, avisar a equipe da previsão de que há lacuna nos dados, para que o treino e as correções não tratem o buraco como consumo zero.

**Atraso de leitura.** A métrica principal é a diferença entre o horário de enqueue do evento mais recente e o horário em que o gateway o publicou. Atraso crescendo costuma ter uma de três causas: broker lento ou recusando publicações, limite de taxa do lado do IoT Hub, ou poucas instâncias para o número de partições. Olhar o broker primeiro, porque é o mais comum.

**Contrapressão do broker.** Se o broker recusar ou demorar, o gateway para de avançar a posição e tenta de novo com espera crescente. Ele não deve descartar mensagens para ganhar velocidade. Aceitar o atraso é melhor do que perder dados, e o agendador de carga tolera dados um pouco velhos melhor do que tolera lacunas.

**Mensagens malformadas.** Payload que não se converte para o formato interno não pode travar a partição inteira. O comportamento é registrar o problema com o identificador do dispositivo, publicar a mensagem num tópico de rejeitadas para inspeção e seguir em frente. Um dispositivo com firmware antigo mandando formato velho foi o caso típico que motivou isso. Cuidado para não registrar o payload completo nos logs, porque ele pode conter dados de consumo de uma residência identificável.

**Privacidade.** Telemetria de uma casa revela rotina de quem mora lá. Os logs do gateway devem trazer identificador de dispositivo e tipo de erro, e não valores de consumo. O acesso aos tópicos do broker interno deve seguir o mesmo cuidado.

**Relógio dos dispositivos.** Como dito acima, o horário do dispositivo pode estar errado. O gateway repassa os dois horários e não corrige nenhum. Corrigir ou descartar por horário implausível é decisão de quem consome, porque cada consumidor tem uma tolerância diferente: a previsão aguenta pouco, o painel aguenta mais.

**Testes.** O ideal é testar com um IoT Hub de desenvolvimento e um broker local, enviando eventos de um dispositivo simulado. Os casos que valem sempre repetir: reinício no meio de um lote (conferir duplicata sem perda), broker indisponível por um tempo, payload malformado e duas instâncias dividindo partições. Teste só com mocks do cliente do Event Hubs já deixou passar erro de posição de leitura, então manter pelo menos um teste de integração real.

## Pendências

- Documentar, para cada consumidor interno, se ele trata duplicatas.
- Definir o alerta de gateway parado com folga em relação à retenção.
- Decidir se o tópico de rejeitadas ganha um consumidor que avise a equipe de suporte dos instaladores quando um dispositivo manda formato inválido de forma repetida.
- Avaliar o caminho cloud-to-device em nota separada, se o produto passar a precisar de comandos para a bateria.
