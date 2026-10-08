---
id: 01KTXDM3G7XVP0D2NNRSX95FAD
created: 2026-06-12T05:01-03:00
---

# Design do instrument-ingest-worker: como ele consome e grava

Anotação rápida sobre o `instrument-ingest-worker`, o serviço que consome as mensagens de saída dos instrumentos e as transforma em dados ligados às entradas do caderno eletrônico. Está incompleta de propósito: escrevi o que lembrei do desenho e deixei de fora tudo que precisaria conferir no código. Onde falo de limites ou valores, quero dizer o valor usual da configuração, não um número fixo.

## O que o worker faz

O `instrument-ingest-worker` é um processo em C# em .NET, rodando como serviço de longa duração. Ele fica ligado a uma fila no RabbitMQ, recebe uma mensagem por arquivo ou lote de saída de instrumento, busca o conteúdo bruto no Azure Blob Storage, valida e grava os metadados no SQL Server. O objetivo final é que o cientista abra a entrada do caderno e veja o resultado do instrumento já associado, e que o responsável de conformidade encontre a trilha de auditoria completa.

Ele não interpreta ciência. Não calcula nada sobre o resultado. Só garante que o que o instrumento produziu chegou inteiro, uma vez só, e com o registro de quem, quando e de onde.

## Consumo da fila

O consumo usa confirmação manual. A mensagem só é confirmada depois que o dado bruto está no blob e o registro está no banco. Se algo falha antes disso, a mensagem volta para a fila ou vai para a fila de mensagens mortas, conforme o tipo de erro.

O número de mensagens em andamento ao mesmo tempo (prefetch) vem da configuração e fica num valor modesto. A ideia é não segurar muitas mensagens não confirmadas se o worker cair, porque cada uma volta a ser entregue e precisa ser tratada como possível duplicata.

Há mais de uma instância do worker em produção. Elas competem pela mesma fila, então nada no código pode assumir ordem global de chegada. A ordem só importa dentro de um mesmo instrumento ou execução, e isso é tratado na lógica de gravação, não na fila.

## Idempotência

Como a entrega é pelo menos uma vez, o worker precisa tolerar a mesma mensagem duas vezes. A chave de deduplicação é montada a partir do identificador do instrumento, da execução e do conteúdo (um hash). Antes de gravar, ele consulta se essa chave já existe. Se existe, confirma a mensagem e registra que foi uma repetição, sem criar nova linha de auditoria de ingestão, só uma anotação de duplicata.

Ponto para revisar depois: a janela entre a verificação e a gravação. Hoje a proteção final é uma restrição de unicidade no banco, e o worker trata a violação como duplicata e não como erro. Vale confirmar que isso continua assim.

## Armazenamento do bruto

O arquivo original vai para o Azure Blob Storage antes de qualquer transformação. O nome do blob é derivado da chave de deduplicação, então reenviar o mesmo conteúdo cai no mesmo lugar. O blob nunca é sobrescrito com conteúdo diferente: se o hash não bate com o existente, isso é tratado como conflito e vai para revisão, não para substituição.

A retenção e a imutabilidade do contêiner são definidas na infraestrutura, não pelo worker. O worker só assume que o que gravou continua lá e igual. Isso importa para a conformidade, porque a trilha de auditoria aponta para esse blob.

## Gravação no SQL Server

Cada ingestão grava, numa única transação, o registro do resultado, o vínculo com a entrada do caderno (quando já dá para resolver) e a linha de auditoria. A linha de auditoria diz que o worker recebeu, de qual mensagem, qual versão do esquema do instrumento foi usada e qual o resultado da validação.

Quando o vínculo com a entrada ainda não pode ser resolvido, por exemplo porque o cientista ainda não criou a entrada, o resultado fica como pendente de associação. Um passo posterior, fora deste worker, tenta de novo. Não deixei isso mais detalhado porque não lembro se a nova tentativa é por agendamento ou por evento.

A transação é curta de propósito. O download e o upload de blobs acontecem fora dela, para não segurar bloqueios no banco enquanto a rede responde.

## Validação e tratamento de erros

Há três categorias de falha, mais ou menos:

- Mensagem malformada ou de instrumento desconhecido: não adianta repetir. Vai direto para a fila de mensagens mortas com o motivo registrado.
- Falha transitória de infraestrutura, como tempo esgotado no banco ou no armazenamento: a mensagem volta para a fila, com espera crescente entre tentativas, até o limite configurado de tentativas.
- Conteúdo que não passa na validação de esquema: o bruto é preservado, o resultado é marcado como rejeitado e a auditoria registra a rejeição. Isso não é descartado em silêncio, porque o responsável de conformidade precisa ver o que foi rejeitado.

Depois de esgotar as tentativas, a mensagem vai para a fila de mensagens mortas. Alguém precisa olhar essa fila; ainda não tenho certeza de quem é o responsável nem se existe alerta. Anotar para perguntar.

## Auditoria e relógio

Os carimbos de tempo na auditoria usam o horário em UTC tomado no momento da gravação, e também guardam o horário que veio do instrumento, quando existe. Os dois são mantidos separados porque o relógio dos instrumentos nem sempre é confiável. Não se corrige o horário do instrumento; se a diferença for grande demais, o worker apenas marca um aviso na validação.

O worker identifica a si mesmo na auditoria com um nome de serviço, não com um usuário humano. Quem consulta a trilha vê claramente que aquela linha veio da ingestão automática.

## Pendências e dúvidas

- Conferir o comportamento exato quando o RabbitMQ perde a conexão no meio de um lote: a recuperação automática de conexão está ligada, mas não verifiquei o que acontece com mensagens não confirmadas em andamento.
- Confirmar o limite de tamanho aceito para um arquivo de instrumento e o que acontece acima dele. Acho que existe um limite configurado, mas não lembro se a rejeição é na fila ou no worker.
- Decidir se vale separar o passo de associação pendente em outro serviço, em vez de deixá-lo perto do worker.
- Ver se existe outra anotação de design sobre o mesmo consumo, para juntar as duas e não deixar versões divergentes.
- Medir a latência do caminho completo em carga normal; só tenho impressão, nenhum dado.
