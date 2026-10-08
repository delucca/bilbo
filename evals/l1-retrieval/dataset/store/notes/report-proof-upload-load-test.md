---
id: 01K7J6KX50CCBXKVT4VD0YVZ67
created: 2025-10-14T17:29-03:00
sources:
  - "doc: Q3 load test report"
---

# Relatório do teste de carga do proof-upload-worker

Rodamos um teste de carga no proof-upload-worker e o resultado principal é este: no pico, ele processou 1200 uploads per minute, com latência mediana de 420 ms. Este relatório guarda esse número e o que dá para concluir dele. Quem abrir o relatório sem contexto deve conseguir responder: quanto o worker aguenta, com que latência, e o que ainda não sabemos.

O proof-upload-worker é a parte do backend do ParcelPin que recebe as provas de entrega capturadas pelos entregadores no app Android e as grava no destino final. O app trabalha offline, então as provas chegam em rajadas, quando o aparelho volta a ter rede. É por isso que o pico importa mais que a média: o tráfego real não é uniforme.

## Resultado do teste

O número que vale guardar:

```text
componente: proof-upload-worker
pico: 1200 uploads per minute
latência mediana: 420 ms
```

Em palavras: no momento de maior carga do teste, o proof-upload-worker tratou 1200 uploads per minute, e metade dos uploads terminou em 420 ms ou menos. A mediana diz o que o upload típico sentiu. Ela não diz nada sobre a cauda, isto é, os uploads mais lentos. Este relatório não registra percentis altos, então não dá para afirmar nada sobre eles. Se alguém precisar dessa informação, o teste tem de ser repetido com essa medição ligada.

O valor de 1200 uploads per minute é o pico observado no teste, não um limite medido de ruptura. Ou seja: o worker aguentou essa carga com a mediana citada, mas o teste não foi desenhado, pelo que está registrado aqui, para descobrir onde ele quebra. Tratar o número como teto seria errado; tratar como "carga já comprovada" é correto.

### O que o número significa na prática

Um upload aqui é o envio de uma prova de entrega de um entregador: os dados da prova e o que o app anexa a ela. Cada upload bem-sucedido é uma entrega que passa a constar no backend. Com o ritmo de pico do teste, o worker consegue absorver uma fila grande de provas acumuladas offline quando muitos entregadores reconectam quase juntos, por exemplo no fim de um turno, quando voltam ao depósito e entram no Wi-Fi.

A latência mediana de 420 ms é a de ponta a ponta no worker, no cenário do teste. Para o entregador, o app envia em segundo plano, então esse tempo quase nunca aparece na tela. Ele importa mais para a fila: se cada upload demora mais, a fila demora mais a esvaziar quando há muitos pendentes.

## Contexto do componente

O app é feito em Kotlin com Android Jetpack. O armazenamento e a sincronização usam Firebase e Cloud Firestore. Os contratos de mensagem entre app e backend usam Protocol Buffers. O proof-upload-worker fica entre o que o app envia e o que o backend guarda, então qualquer mudança em esquema, em regras de segurança ou em índices do Firestore pode mexer no desempenho que o teste mediu.

Por isso o resultado vale para a configuração em que o teste rodou. Se mudarmos o formato das mensagens, a forma de gravar no Cloud Firestore ou a quantidade de dados por prova, o número antigo deixa de ser garantia e o teste precisa ser refeito.

### Pontos que costumam pesar no desempenho

Esta lista é de hipóteses de trabalho, não de achados do teste. Nada abaixo foi medido separadamente.

- Tamanho de cada prova: provas maiores custam mais em rede e em gravação. Se o app passar a enviar mais dados por entrega, a vazão por minuto tende a cair.
- Gravações no Cloud Firestore: há limites de taxa por documento e por coleção. Se muitas provas tocarem o mesmo documento, a contenção pode aparecer antes do limite do worker.
- Serialização com Protocol Buffers: costuma ser barata, mas campos novos e mensagens aninhadas grandes aumentam o trabalho de decodificação.
- Repetições do app: como o app trabalha offline e reenvia quando falha, o mesmo upload pode chegar mais de uma vez. O worker precisa continuar idempotente para que repetição não vire duplicata nem carga inútil.
- Rajadas de reconexão: o tráfego real é concentrado em horários, não espalhado. O pico do teste é a referência certa para isso, mas é preciso comparar com o pico real em produção.

## Limites do teste e o que falta

O que este relatório não cobre, para ninguém tirar conclusão a mais:

1. Não há medição registrada de percentis altos de latência. Só a mediana de 420 ms está registrada.
2. Não há ponto de ruptura. O teste mostra que o proof-upload-worker sustentou 1200 uploads per minute no pico, e só isso.
3. Não há comparação com o tráfego real de produção neste relatório. Falta confrontar o pico do teste com o maior pico que os entregadores geram de fato.
4. Não há registro de comportamento sob falha parcial, como o Firestore lento ou indisponível por um tempo, ou perda de rede no meio de um upload.
5. Não há registro de custo. Um volume maior de gravações e leituras no Firebase tem custo, e ele deve ser olhado antes de qualquer aumento de carga.

### Próximos passos sugeridos

Em ordem de utilidade:

- Repetir o teste medindo percentis altos, para saber como é a cauda e não só o meio.
- Subir a carga acima de 1200 uploads per minute até algo degradar, e anotar o que degrada primeiro: o worker, o Firestore ou a rede.
- Rodar um cenário de falha: Firestore lento, reenvios em massa, mensagens repetidas, para confirmar a idempotência sob pressão.
- Comparar com os dados reais de produção, para saber quanta folga existe entre o pico do teste e o pico verdadeiro.
- Guardar a configuração do teste junto com o resultado, para que a próxima pessoa saiba o que foi medido e possa repetir.

## Como usar este número

Para planejamento de capacidade, use 1200 uploads per minute como o pico já demonstrado e 420 ms como a mediana associada. Não use como promessa a clientes sem antes cobrir os itens da seção anterior. Para alertas, a mediana ajuda a definir um valor normal, mas um alerta útil precisa de um percentil alto, e esse dado ainda não existe.

Se alguém mudar o proof-upload-worker, o esquema em Protocol Buffers ou a forma de gravar no Cloud Firestore, vale refazer o teste e atualizar este relatório em vez de criar outro, porque o assunto é o mesmo: o desempenho do proof-upload-worker sob carga.

### Resumo curto para quem tem pressa

O proof-upload-worker foi testado sob carga. No pico tratou 1200 uploads per minute e a mediana de latência foi 420 ms. Não sabemos ainda a cauda de latência, nem onde ele quebra, nem como se comporta sob falha. Esses três pontos são o que falta medir.

## Observações finais

O teste dá confiança de que o caminho de upload de provas aguenta uma rajada de reconexão considerável. Isso importa porque o valor do ParcelPin depende de a prova de entrega chegar ao backend mesmo depois de horas offline. Um entregador que perde a prova perde a defesa numa disputa de entrega, então atraso na fila é incômodo, mas perda é um problema sério.

Por isso a prioridade, depois deste teste, não é só ir mais rápido. É saber com segurança que nenhuma prova se perde nem se duplica quando a carga sobe ou quando algo falha no meio. A vazão medida é boa; a garantia de entrega sob falha ainda precisa de evidência própria.

Quando o teste for repetido, registrar junto: o que foi medido, com quais dados de entrada, em que ambiente, e quais métricas foram coletadas. Sem isso, o próximo número não é comparável a este e a discussão volta ao começo.

Anotação para quem for ler depois: se o número de 1200 uploads per minute aparecer em outro documento sem a mediana de 420 ms ao lado, desconfie. Os dois valores só fazem sentido juntos, porque vazão sem latência esconde se o worker estava sofrendo para acompanhar.
