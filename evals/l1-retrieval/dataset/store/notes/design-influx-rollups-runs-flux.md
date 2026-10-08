---
id: 01JVKVSB8BC2QECRGN8WAX8T9Z
created: 2025-05-19T05:50-03:00
sources:
  - "code: tasks/rollup_5m.flux"
---

# Design do influx-rollups

O `influx-rollups` é o componente do GridHaven que reduz a resolução da telemetria bruta guardada no InfluxDB. Ele roda como a task Flux `rollup_5m`, a cada 5 minutos, e grava a potência média no bucket `telemetry_5m`. Esta nota descreve o desenho, as razões e os cuidados. Foi escrita com pressa, para quem for mexer nisso depois e não quiser reconstruir o raciocínio.

Se você encontrar o nome `downsampler` em código antigo, em dashboards, em issues ou em conversas de instaladores, é o mesmo componente. `downsampler` era o nome anterior do `influx-rollups`; hoje o nome certo é `influx-rollups`. Ao tocar em algo que ainda usa o nome velho, vale renomear, mas sem quebrar referências externas.

## Contexto e motivação

Os inversores e medidores das casas publicam leituras via MQTT. Elas passam pelo Azure IoT Hub e acabam no InfluxDB em alta frequência. Esse dado bruto é ótimo para diagnóstico, mas pesado para tudo que o GridHaven faz no dia a dia: a previsão de produção solar, feita em Julia, e o agendamento de carga da bateria contra as tarifas por horário de uso.

Nenhum desses consumidores precisa de resolução fina. A previsão trabalha com janelas curtas, mas não com cada leitura individual. O painel em Svelte que o cliente final vê também fica mais rápido e barato lendo dados já agregados. Por isso existe uma camada de rollup: uma cópia agregada, menor, que serve a maior parte das consultas.

## O que o componente faz

A task `rollup_5m` lê as leituras brutas de potência da janela mais recente, calcula a média por série em janelas de 5 minutos e escreve o resultado no bucket `telemetry_5m`. O agendamento é a cada 5 minutos, alinhado com o tamanho da janela, de modo que cada execução fecha uma janela por série.

É só a média de potência. Não há máximo, mínimo nem contagem nesse rollup. Se alguém precisar de picos, isso é uma decisão nova e merece outra task ou outro bucket, não um acréscimo silencioso aqui.

## Entrada e saída

A entrada é o bucket de telemetria bruta no InfluxDB, alimentado pelo caminho MQTT e IoT Hub. A saída é o bucket `telemetry_5m`. As tags que identificam a instalação e o dispositivo são preservadas, para que as consultas por casa continuem funcionando sem junção extra.

O bucket de saída tem retenção própria, bem mais longa que a do bruto. Essa é a ideia central: o bruto expira cedo e o agregado fica para histórico, treino de modelo e relatórios dos instaladores. Os valores exatos de retenção ficam na configuração do InfluxDB, não aqui, para esta nota não envelhecer.

## Decisões de desenho

Primeiro, usar uma task nativa do InfluxDB em Flux, em vez de um serviço externo. Menos peças para operar, sem processo para monitorar além do próprio banco, e o dado não sai do InfluxDB para ser agregado.

Segundo, média e não último valor. Para energia e previsão, a média da janela representa melhor o que a casa produziu ou consumiu. O último valor seria sensível a ruído e a atrasos de entrega.

Terceiro, uma única resolução de rollup por enquanto. Mais níveis aumentam o custo de manutenção e a chance de inconsistência entre eles. Só vale criar outro se aparecer consumidor real.

## Janelas e alinhamento

As janelas são alinhadas ao relógio, não ao momento em que a task dispara. Assim duas execuções nunca produzem janelas deslocadas entre si, e reexecutar o mesmo intervalo gera os mesmos pontos. Isso importa para a idempotência descrita abaixo.

A task não deve agregar a janela ainda aberta. Ela olha para trás, para janelas já completas, com uma pequena folga para absorver atraso de chegada. Essa folga é um compromisso: quanto maior, mais tarde o dado agregado aparece; quanto menor, mais risco de média calculada com leituras faltando.

## Dados atrasados e lacunas

Dispositivos domésticos perdem conexão. Quando voltam, o IoT Hub entrega leituras antigas em lote. Se elas chegarem depois que a janela foi agregada, a média em `telemetry_5m` fica calculada com dados incompletos.

Hoje a abordagem é aceitar isso para atrasos pequenos e fazer um reprocessamento manual do intervalo quando a lacuna for grande. Como a escrita é idempotente, reprocessar sobrescreve os pontos antigos com os corretos. Janelas sem nenhuma leitura não geram ponto; a ausência é o sinal, e quem consome deve tratar buracos explicitamente em vez de supor zero.

## Idempotência e reprocessamento

Um ponto no InfluxDB com o mesmo conjunto de tags e o mesmo timestamp substitui o anterior. Como as janelas são alinhadas, rodar a `rollup_5m` duas vezes sobre o mesmo período produz o mesmo resultado, sem duplicar. Isso torna o backfill seguro.

Para reprocessar, execute a mesma lógica da task sobre o intervalo desejado, escrevendo no mesmo bucket. Faça em trechos moderados para não competir com a carga normal do banco. Avise quem opera os painéis, porque os gráficos podem mudar retroativamente.

## Consumidores

O serviço de previsão em Julia lê de `telemetry_5m` para montar séries de treino e para ajustar a previsão com o que foi realmente produzido. O agendador de bateria usa o mesmo bucket para comparar o previsto com o realizado e corrigir o plano de carga frente às tarifas.

O frontend em Svelte consulta o agregado para os gráficos de histórico. Para o tempo real, o painel ainda pode usar o dado bruto recente, mas qualquer visão de período maior deve vir do bucket agregado. Se um consumidor novo consultar o bruto por períodos longos, é sinal de que algo está errado.

## Operação e monitoramento

O que olhar quando algo parece estranho: se a task `rollup_5m` está rodando no ritmo esperado, se o último ponto em `telemetry_5m` é recente e se há falhas registradas no histórico de execuções da task no InfluxDB. Uma task que falha em silêncio é o pior caso, porque os consumidores continuam lendo dados velhos sem erro.

Vale ter um alerta de frescor: se o ponto mais recente de uma instalação ativa estiver velho demais, avisar. Distinguir entre dispositivo offline e task parada é o primeiro passo do diagnóstico; se todas as instalações param ao mesmo tempo, o problema é a task ou o banco.

## Limitações conhecidas

Só existe a média. Atrasos grandes exigem intervenção manual. Não há versionamento do rollup, então mudar a lógica da média afeta o histórico só se alguém reprocessar. E a task é única: se ela ficar lenta com o crescimento do número de instalações, o próprio agendamento pode atrasar.

Outro ponto é a mistura de nomes. Em material antigo ainda aparece `downsampler`, o que confunde quem chega agora. Buscas em logs e dashboards devem considerar os dois nomes.

## Pendências e próximos passos

Avaliar se vale dividir a carga da task por grupos de instalações, caso o volume cresça. Decidir uma política formal para dados atrasados, em vez de depender de reprocessamento manual. Revisar a retenção do `telemetry_5m` com os instaladores, que usam o histórico em relatórios. E limpar de vez as referências restantes ao nome `downsampler` em documentação e painéis, mantendo apenas `influx-rollups`.

Se algo disso mudar o formato do dado agregado, avisar antes quem mantém a previsão em Julia e o frontend, porque ambos dependem da forma atual do bucket.
