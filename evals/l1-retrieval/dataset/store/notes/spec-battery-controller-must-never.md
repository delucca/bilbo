---
id: 01JVBCKAN0BQA1D2VFJ4P6SY49
created: 2025-05-15T22:51-03:00
---

# battery-controller: especificação do limite de potência de carga

Esta nota registra a regra central do `battery-controller`: ele nunca pode comandar uma potência de carga acima do limite do inversor, que é de `5000 W`. Qualquer comando de carga que saia do componente tem de respeitar esse teto, não importa o que o agendamento, a previsão solar ou a tarifa peçam.

## Contexto

O GridHaven prevê a geração solar doméstica (em Julia) e agenda a carga da bateria conforme a tarifa horária (time-of-use). O `battery-controller` é a parte que transforma o plano de carga em comandos concretos para o inversor. O plano diz quanto e quando carregar; o `battery-controller` decide o que de fato é enviado. Por isso o limite fica aqui, no último ponto antes do dispositivo, e não só no planejador.

O inversor tem um limite físico de carga de `5000 W`. Passar disso pode fazer o inversor recusar o comando, limitar por conta própria de forma imprevisível ou, no pior caso, entrar em falha. Não vale confiar que o equipamento vai se proteger sozinho.

## Regra

- O `battery-controller` não comanda carga acima de `5000 W`, em nenhuma circunstância.
- Se o plano pedir mais que isso, o valor é reduzido para o teto antes de sair. Não se descarta o comando inteiro.
- O corte vale para todo caminho que gera comando: agendamento normal, recarga antecipada para uma janela de tarifa cara, e comandos manuais vindos do app do instalador ou do cliente.
- O limite se aplica ao valor final enviado, depois de qualquer soma ou ajuste feito dentro do componente.

## Onde aplicar o limite

A checagem deve ficar na última etapa antes da publicação do comando, depois de todos os cálculos. Se ficar no início, uma etapa posterior que some margens ou correções pode estourar o teto sem ninguém notar. Regra prática: o clamp é a última operação sobre o valor de potência.

O planejador também pode conhecer o limite para gerar planos viáveis, mas isso é otimização. A garantia de segurança é a do `battery-controller`.

## Exemplo

Trecho ilustrativo do clamp em Julia, só para mostrar a ideia:

```julia
const LIMITE_INVERSOR_W = 5000

limitar_carga(pedido_w) = clamp(pedido_w, 0, LIMITE_INVERSOR_W)
```

O limite inferior é zero porque este comando é de carga; descarga segue outro caminho e outras regras.

## Comandos e telemetria

Os comandos saem por MQTT, passando pelo Azure IoT Hub até o dispositivo. A telemetria de potência medida volta pelo mesmo caminho e é gravada no InfluxDB. Quando um pedido for reduzido pelo teto, vale registrar isso em log com o valor pedido e o valor enviado. Assim dá para ver depois, na série temporal, que o plano estava pedindo mais do que o inversor aguenta.

## Casos de borda

- Pedido negativo: não é um comando de carga válido; tratar como zero ou rejeitar, nunca converter em carga.
- Valor ausente ou não numérico: não enviar nada e registrar o erro. Nunca assumir o teto como padrão.
- Instalações com mais de um inversor: o limite vale por inversor, a menos que uma nota futura diga outra coisa. Hoje o que está especificado é o teto do inversor.
- Mudança de modelo de inversor: o teto deve vir de configuração por instalação, mas o valor de referência desta especificação continua sendo `5000 W`.

## Testes esperados

1. Pedido abaixo do teto passa sem alteração.
2. Pedido exatamente no teto passa sem alteração.
3. Pedido acima do teto sai reduzido ao teto.
4. Pedido negativo ou inválido não gera carga.
5. Comando manual acima do teto também é reduzido.

Os testes devem olhar o valor publicado, não só o valor retornado pela função de clamp, para pegar etapas posteriores que alterem a potência.

## Pendências

- Confirmar se o teto deve ser lido de configuração da instalação ou fixo no código por enquanto.
- Decidir se o app Svelte deve avisar o usuário quando um comando manual for reduzido.
- Definir o alerta para reduções frequentes, que indicam plano mal dimensionado.
