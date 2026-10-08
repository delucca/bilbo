---
id: 01M1S1RXEWTJV4KT99A4DHXPNN
created: 2026-09-05T12:07-03:00
---

# Especificação de telemetry-ingest: tolerância a desconexão do broker MQTT

Esta nota substitui a nota anterior "telemetry ingest must tolerate". O novo valor é este: `telemetry-ingest` deve tolerar uma desconexão do broker MQTT de até `300 seconds` sem perder leituras. O valor antigo deixa de valer e não deve ser usado em testes, alertas nem na documentação para instaladores.

## Contexto

O GridHaven prevê a produção solar residencial e agenda a carga da bateria contra tarifas por horário de uso. Tudo isso depende de leituras que chegam das casas de forma contínua. O `telemetry-ingest` é o componente que recebe essas leituras pelo MQTT, valida o que chegou e grava na série temporal, no InfluxDB. Se faltar leitura, a previsão em Julia trabalha com buraco, e o agendamento da bateria pode escolher a hora errada para carregar.

O broker é o ponto mais frágil do caminho. Ele reinicia em manutenção, perde rede e sofre com falhas do serviço gerenciado. Por isso a tolerância a desconexão precisa ser um requisito explícito, com número, e não uma esperança.

## O requisito

O requisito, escrito de forma que dê para testar: se a conexão entre o `telemetry-ingest` e o broker MQTT ficar fora por até `300 seconds`, nenhuma leitura gerada nesse intervalo pode ser perdida. Quando a conexão voltar, as leituras do intervalo têm de chegar ao InfluxDB com o carimbo de tempo original da medição, e não com a hora da reconexão.

O limite vale para uma desconexão contínua. Se a queda passar de `300 seconds`, o sistema não promete mais nada sobre as leituras além desse ponto, mas precisa avisar que passou do limite. Ver a seção de comportamento acima do limite.

```text
telemetry-ingest:
  broker_disconnect_tolerance: 300 seconds
  on_reconnect: replay_buffered_readings
```

O bloco acima é só uma ilustração do requisito. O nome real da chave de configuração deve seguir o que o código já usa.

## O que mudou em relação à nota anterior

A nota anterior tinha uma tolerância menor. Ela foi substituída porque o valor antigo não cobria as quedas reais de broker que aconteceram em campo. O novo valor é `300 seconds`. Quem encontrar o valor antigo em código, testes, painéis ou texto de documentação deve tratar como resíduo e atualizar.

Não mexemos no que o componente faz com a leitura depois que ela é aceita. A mudança é só sobre quanto tempo de queda o componente aguenta sem perder dado.

## Escopo

Entra no escopo: a conexão do `telemetry-ingest` com o broker MQTT, a retenção temporária das leituras durante a queda, o reenvio na reconexão e a gravação ordenada no InfluxDB. Também entra a sinalização quando o limite é ultrapassado.

Fica fora do escopo: o caminho entre o dispositivo da casa e o broker, a lógica de previsão em Julia, o agendamento da bateria e a interface em Svelte. Se o dispositivo da casa perde a rede dele, isso é outro problema, tratado no firmware e no Azure IoT Hub, e não conta para esta tolerância.

## Comportamento durante a desconexão

Assim que o cliente MQTT detecta a queda, o `telemetry-ingest` entra em modo de espera com reconexão automática. Ele tenta voltar ao broker repetidamente, com intervalo crescente entre as tentativas, mas com um teto para esse intervalo, de modo que a reconexão aconteça logo depois que o broker voltar e não muito tempo depois.

Durante a queda, o componente continua aceitando tudo o que ainda chega por outros caminhos, se houver, e guarda essas leituras no buffer. Ele não pode travar nem reiniciar sozinho por causa da queda. Reiniciar zeraria o buffer em memória e quebraria a promessa.

## Buffer e retenção

O buffer precisa guardar pelo menos o volume de leituras que se acumula em `300 seconds` na carga máxima esperada. O dimensionamento deve partir do número de instalações ativas e da frequência de amostragem, e deve ter folga. Cortar o buffer pelo tamanho antes de cobrir o tempo exigido viola o requisito.

Se o buffer ficar só em memória, um reinício do processo no meio da queda perde dados. A decisão de usar persistência local em disco ou aceitar esse risco ainda precisa ser registrada. Até lá, a regra é: o processo não reinicia de propósito durante uma desconexão, e implantações novas esperam a conexão voltar.

## Reconexão e reenvio

Na reconexão, o componente reassina os tópicos necessários e esvazia o buffer na ordem de medição. A gravação no InfluxDB deve ser idempotente: se uma leitura for enviada duas vezes, o resultado é o mesmo ponto, e não um duplicado. Isso importa porque o reenvio e as leituras novas chegam ao mesmo tempo.

O componente deve limitar a velocidade do reenvio para não derrubar o InfluxDB nem atrasar as leituras novas. As leituras atuais têm prioridade sobre o histórico acumulado, desde que o histórico termine de ser gravado em tempo razoável.

## Comportamento acima do limite

Se a queda passar de `300 seconds`, o `telemetry-ingest` registra um erro claro nos logs, marca o intervalo afetado como incompleto e expõe isso numa métrica. Ele não deve inventar valores para preencher o buraco. A previsão em Julia precisa saber que existe um intervalo sem dado confiável, para tratar como ausente.

Depois de passar do limite, o componente continua tentando reconectar e continua guardando o que for possível. O limite define a garantia, não o ponto em que ele desiste.

## Ordem e carimbos de tempo

O carimbo de tempo que vale é o da medição, vindo do dispositivo. O horário em que o `telemetry-ingest` recebeu a leitura serve só para diagnóstico. Leituras fora de ordem são normais depois de uma reconexão e não podem ser descartadas por parecerem antigas.

Se o relógio do dispositivo estiver claramente errado, a leitura é marcada como suspeita, não apagada. A decisão de corrigir ou ignorar fica a cargo de quem consome o dado.

## Como testar

O teste principal derruba o broker, mantém a queda por `300 seconds`, gera leituras o tempo todo e confere que, depois da volta, todas estão no InfluxDB com o carimbo certo e sem duplicatas. Um segundo teste faz a mesma coisa com queda menor, e um terceiro passa do limite e confere o erro e a métrica de intervalo incompleto.

Também vale testar quedas repetidas em sequência, com pouco tempo entre elas, porque o buffer pode estar parcialmente cheio quando a próxima queda começa. A soma das quedas seguidas não deve esconder perda silenciosa.

## Observabilidade

O componente deve expor, no mínimo: se está conectado ao broker, há quanto tempo está desconectado, quantas leituras estão no buffer, quantas foram reenviadas e quantas foram perdidas por passar do limite. O alerta para o time deve disparar antes de a queda alcançar o limite, e não só depois dele, para dar tempo de agir.

Os painéis devem mostrar a tolerância atual de `300 seconds` como referência, para que fique visível o quanto de margem sobra durante uma queda em andamento.

## Pontos em aberto

Falta decidir se o buffer terá persistência em disco. Falta confirmar o dimensionamento do buffer com a carga real das instalações. Falta alinhar com quem cuida do Azure IoT Hub se a política de retenção deles interfere no reenvio. Quando cada ponto for fechado, atualizar esta nota em vez de criar outra sobre o mesmo assunto.

## Resumo para quem chega agora

O `telemetry-ingest` aguenta até `300 seconds` de broker MQTT fora do ar sem perder leitura. Guarda, reconecta, reenvia em ordem e grava sem duplicar no InfluxDB. Passou do limite, avisa e marca o buraco. Este valor substitui o da nota anterior.
