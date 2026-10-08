---
id: 01M3R5S2G9FMVG2FBK6J5M34Y1
created: 2026-09-30T00:29-03:00
---

# instrument-ingest-worker: prefetch reduzido para 8

Esta nota substitui a nota anterior sobre "instrument ingest worker rabbitmq" e fixa o novo valor: o `instrument-ingest-worker` passa a usar um prefetch count of 8, no lugar do valor anterior de 20.

A razão é simples e já foi vista em operação: a memória do processo continuava disparando quando chegavam arquivos de espectro de 2 GB. Reduzir o prefetch foi a mudança mais barata que resolveu o problema sem mexer no formato das mensagens nem no fluxo de auditoria. O resto da nota explica o contexto, o que foi considerado, o que muda para quem opera o serviço e como conferir se a decisão continua valendo.

## Contexto

O LabNotebook Sync liga as entradas do caderno eletrônico de laboratório à saída bruta dos instrumentos e mantém a trilha de auditoria de tudo que acontece nesse caminho. Quem usa são cientistas de pesquisa, que querem ver o resultado do instrumento anexado à entrada certa, e responsáveis de conformidade, que querem provar depois quem registrou o quê e quando. O `instrument-ingest-worker` fica no meio desse caminho.

O papel dele é consumir mensagens do RabbitMQ que anunciam que um instrumento terminou uma corrida e produziu saída. Para cada mensagem, o worker busca o conteúdo, grava no Azure Blob Storage, registra os metadados e o vínculo com a entrada do caderno no SQL Server e só então confirma a mensagem (ack). A trilha de auditoria depende desse registro: se o ack acontece antes da gravação terminar, a trilha pode ficar com um furo, e se a gravação acontece sem ack, a mensagem volta e o worker precisa tolerar a repetição.

O worker é escrito em C# sobre .NET. Ele roda como serviço de longa duração, com um consumidor assíncrono por instância. O comportamento de memória dele depende muito de quantas mensagens ele está processando ao mesmo tempo, e isso é controlado, do lado do broker, pelo prefetch.

### O que o prefetch controla

O prefetch (o QoS por consumidor no RabbitMQ) define quantas mensagens sem ack o broker entrega ao consumidor de uma vez. Enquanto o consumidor não confirma, o broker não manda além do limite. Na prática, o prefetch é o teto de trabalho em andamento por consumidor. Se cada unidade de trabalho segura um arquivo grande na memória, o teto de memória é aproximadamente o prefetch multiplicado pelo tamanho típico do arquivo mais a sobrecarga de cada etapa.

Por isso um valor que parece razoável para mensagens pequenas pode ser perigoso quando o corpo da mensagem aponta para arquivos grandes. O valor antigo, 20, foi escolhido pensando em saídas de instrumento pequenas e frequentes. Ele não considerava bem os arquivos de espectro grandes.

## Decisão

O `instrument-ingest-worker` usa agora um prefetch count of 8. O valor anterior era 20. A mudança vale para o consumidor da fila de ingestão de saída de instrumentos e é aplicada na configuração do serviço, não no código de negócio.

Resumo do que ficou decidido:

- O prefetch do consumidor do `instrument-ingest-worker` é 8.
- O valor antigo, 20, está aposentado e não deve voltar sem uma nova decisão registrada.
- O motivo é a memória que continuava subindo de forma brusca com arquivos de espectro de 2 GB.
- Nenhuma outra configuração do consumidor foi alterada junto com esta, de propósito, para que o efeito possa ser atribuído só ao prefetch.

O trecho abaixo mostra a forma da configuração, só com valores desta decisão. O nome exato da chave depende de como a configuração do serviço está organizada no repositório; aqui ele é ilustrativo.

```
instrument-ingest-worker
  prefetch: 8    # antes: 20
```

## Por que mudar

O problema observado foi pico de memória. Mesmo depois de outros ajustes anteriores no worker, a memória continuava a disparar quando vários arquivos de espectro de 2 GB estavam em processamento ao mesmo tempo. Com o prefetch antigo, o broker entregava ao mesmo consumidor um número alto de mensagens sem ack, e se várias delas apontavam para arquivos grandes, o processo tentava lidar com todas em paralelo.

O efeito para quem opera era ruim de duas formas. A primeira é o risco de o processo ser encerrado por falta de memória pelo ambiente de execução, o que derruba também o trabalho pequeno que estava em andamento. A segunda é a pressão de coleta de lixo do .NET, que deixa a latência de todo o consumidor pior mesmo quando o processo não chega a cair. Em ambos os casos, mensagens sem ack voltam para a fila, e o worker repete trabalho que já tinha avançado parte do caminho.

Como a gravação no Azure Blob Storage e o registro no SQL Server precisam ser idempotentes de qualquer forma, a repetição não corrompe dados. Mesmo assim, ela gera ruído na trilha de auditoria, porque o mesmo evento aparece tentado mais de uma vez, e isso incomoda a conformidade na hora de ler o histórico.

### Por que não foi só um problema de tamanho de arquivo

A primeira reação foi tratar o arquivo grande como o problema e pensar em processá-lo em pedaços. Isso continua sendo uma boa ideia de longo prazo, mas é uma mudança maior, que mexe no caminho de leitura e de gravação e precisa de testes cuidadosos com os dados reais. O prefetch resolve o sintoma agora, sem tocar nesse caminho. Os dois não se excluem: reduzir o prefetch não impede que a leitura em fluxo seja feita depois, e se ela for feita, o valor de 8 pode ser revisto.

## Alternativas consideradas

Antes de fixar o valor, foram olhadas as opções abaixo. A escolha final foi reduzir o prefetch, e o restante fica registrado para que ninguém precise refazer a conversa.

### Manter o prefetch antigo e aumentar a memória disponível

Dar mais memória ao serviço adia o problema, não o remove. O pico depende de quantos arquivos grandes chegam juntos, e isso não tem limite natural. Além disso, o custo de memória maior se paga o tempo todo, inclusive quando só chegam arquivos pequenos. Foi descartado.

### Limitar por tamanho em vez de por contagem

O RabbitMQ limita o prefetch por contagem de mensagens e, em algumas configurações, por tamanho em bytes do que está pendente, mas o corpo da nossa mensagem é só um aviso com referência ao arquivo, então o tamanho da mensagem não representa o tamanho do trabalho. Um limite por bytes no broker não ajudaria. Um limite por tamanho real do arquivo teria de ser feito dentro do worker, com um semáforo ou algo parecido, e isso é código novo. Fica como ideia futura.

### Separar filas por tamanho esperado

Uma fila para saídas pequenas e outra para arquivos grandes permitiria prefetch alto numa e baixo na outra. É a solução mais limpa, mas exige que quem publica a mensagem saiba ou estime o tamanho, e hoje nem todo instrumento informa isso de forma confiável. Também muda a topologia do RabbitMQ, o que pede coordenação com quem cuida do broker. Não foi feito agora.

### Reduzir o prefetch

É a opção escolhida. Muda um valor de configuração, pode ser revertida em minutos, não altera contrato de mensagem e tem efeito direto sobre o teto de trabalho em andamento. O preço é vazão potencialmente menor, discutido adiante.

### Por que 8 e não outro valor

O valor 8 veio de tentativa guiada pela observação, não de um cálculo fechado. Foi o ponto em que a memória deixou de disparar com arquivos de 2 GB sem derrubar de forma visível a vazão para arquivos pequenos. Quem quiser mexer de novo deve partir de medição, não de palpite, e registrar o novo valor aqui, substituindo esta decisão em vez de abrir uma nota paralela.

## Efeitos esperados e custo

O efeito principal é um teto de memória mais previsível. Com no máximo 8 mensagens sem ack por consumidor, o pior caso de arquivos grandes simultâneos fica bem abaixo do que era possível antes. O comportamento esperado é que os picos que apareciam com arquivos de 2 GB desapareçam ou fiquem pequenos o bastante para o serviço absorver.

O custo é a vazão. Com menos mensagens em voo, o consumidor passa mais tempo esperando o próximo ack do que antes, e isso pesa mais quando cada mensagem é rápida e a latência de rede até o broker é relevante. Para o fluxo normal de saídas pequenas, a diferença deve ser modesta, porque o gargalo costuma estar na gravação do blob e no banco, não na entrega pelo broker. Se mesmo assim a fila começar a acumular, a resposta preferida é aumentar o número de instâncias do worker, não voltar ao prefetch antigo, já que cada instância nova traz o seu próprio teto de memória previsível.

### Relação com a escala horizontal

O prefetch é por consumidor. Duas instâncias do worker somam os limites delas. Então o trabalho total em voo no sistema é o prefetch vezes o número de consumidores, e a memória total segue o mesmo raciocínio, só que distribuída em processos diferentes. Isso é bom: um pico numa instância não derruba as outras. Quando o laboratório crescer, a escala deve vir do número de instâncias.

### Relação com a trilha de auditoria

A decisão não muda quando o ack é enviado. Continua valendo a regra de só confirmar a mensagem depois que o blob foi gravado e o registro no SQL Server foi concluído. O prefetch menor só reduz o número de eventos em andamento ao mesmo tempo. Para a conformidade, isso torna a trilha mais fácil de ler, porque há menos tentativas duplicadas causadas por reentrega depois de queda do processo.

## Como verificar

A decisão só é boa se a memória realmente se comportar. Estas são as conferências que valem a pena depois de qualquer implantação que mexa no worker.

1. Conferir na configuração implantada que o prefetch do `instrument-ingest-worker` é 8 e não 20. Uma configuração antiga em algum ambiente esquecido é a causa mais provável de o problema voltar.
2. Olhar no painel de gerenciamento do RabbitMQ o número de mensagens sem ack por consumidor. Ele não deve passar do prefetch. Se passar, há mais de um canal ou mais de um consumidor no mesmo processo.
3. Observar a memória do processo durante a chegada de arquivos de espectro de 2 GB. O comportamento esperado é uma subida limitada que desce depois do ack, sem degraus que continuem crescendo.
4. Observar a profundidade da fila de ingestão em horário de pico. Crescimento sustentado indica que falta capacidade, e a resposta é escalar instâncias.
5. Procurar na trilha de auditoria registros de tentativa repetida para o mesmo evento. Poucos ou nenhum é o esperado depois da mudança.

### Sinais de que a decisão precisa ser revista

- A fila acumula mesmo com instâncias suficientes e o gargalo comprovado é a espera por ack, não a gravação.
- Passam a existir arquivos bem maiores que os de 2 GB, e mesmo 8 em paralelo estoura a memória.
- O worker passa a ler os arquivos em fluxo, sem carregá-los inteiros, e o teto de memória deixa de depender do prefetch.
- A topologia muda para filas separadas por tamanho, o que permitiria valores diferentes por fila.

Em qualquer desses casos, a nota deve ser atualizada com o novo valor e o novo motivo, e não duplicada.

## Riscos e pontos de atenção

### Configuração divergente entre ambientes

O risco mais concreto é o valor antigo continuar vivo em algum lugar: um arquivo de configuração de ambiente de teste, um parâmetro de implantação, uma variável herdada. Quando a memória voltar a disparar, olhe primeiro isso. Vale conferir também se o código aplica o prefetch ao canal certo e antes de iniciar o consumo, porque no cliente .NET do RabbitMQ a definição de QoS precisa vir antes de o consumidor começar a receber mensagens para valer desde o início.

### Mais de um consumidor no mesmo processo

Se o serviço for alterado para abrir vários consumidores em um único processo, cada um terá seu próprio prefetch, e a soma pode reproduzir o problema antigo. Qualquer mudança que aumente concorrência dentro do processo deve ser avaliada contra o teto de memória, não só contra a vazão.

### Reentrega e idempotência

O prefetch menor reduz a reentrega em caso de queda, mas não a elimina. A idempotência da gravação no Azure Blob Storage e do registro no SQL Server continua sendo requisito. Não se deve relaxar essa garantia por causa desta mudança, e testes que simulam queda no meio do processamento continuam relevantes.

### Confusão entre prefetch e paralelismo interno

Entre quem lê o código é comum confundir prefetch com o número de tarefas que o worker executa de fato ao mesmo tempo. São coisas diferentes: o prefetch é o que o broker deixa em voo, e o paralelismo interno é o que o worker decide processar. Se o paralelismo interno for menor que o prefetch, o resto fica na memória do cliente esperando vez, ocupando espaço sem produzir nada. Por isso a escolha de manter o prefetch baixo também evita esse acúmulo ocioso.

### Efeito sobre outros consumidores da mesma fila

Se outro serviço consumir da mesma fila, a mudança não o afeta, pois o prefetch é configurado por consumidor. Mas a distribuição de mensagens entre consumidores passa a ser mais equilibrada, já que nenhum deles acumula uma fila grande de mensagens sem ack enquanto os outros ficam ociosos. É um efeito colateral bom e esperado.

## Histórico e o que fica pendente

A nota anterior, sobre o worker de ingestão e o RabbitMQ, registrava o prefetch antigo e as suposições da época: arquivos pequenos, chegada frequente, memória sem pressão. Essas suposições deixaram de valer quando passaram a entrar arquivos de espectro grandes, de 2 GB. Esta nota a substitui. Quem encontrar a nota antiga deve considerá-la superada e seguir esta.

O que ficou pendente, sem prazo definido:

- Avaliar leitura e gravação em fluxo, para que o consumo de memória não dependa do tamanho do arquivo.
- Avaliar filas separadas por tamanho esperado, caso os instrumentos passem a informar o tamanho de forma confiável.
- Avaliar um limite interno por tamanho real do arquivo, dentro do worker, como complemento ao prefetch.
- Revisar o painel de monitoramento para que a memória por instância e as mensagens sem ack apareçam lado a lado, o que facilita a conferência descrita acima.

Nada disso é urgente enquanto o prefetch count of 8 mantiver a memória estável. Se a memória voltar a disparar com este valor, a hipótese a testar primeiro é configuração divergente ou concorrência extra dentro do processo, e só depois reduzir ainda mais o prefetch.

## Resumo para consulta rápida

- Componente: `instrument-ingest-worker`.
- Decisão: prefetch count of 8.
- Valor anterior: 20, substituído.
- Motivo: a memória continuava disparando com arquivos de espectro de 2 GB.
- Custo aceito: vazão potencialmente menor por consumidor; a compensação é escalar instâncias.
- Não mudou: o momento do ack, que vem depois da gravação no Azure Blob Storage e do registro no SQL Server.
- Onde olhar se o problema voltar: configuração implantada, número de consumidores no processo e mensagens sem ack no RabbitMQ.
