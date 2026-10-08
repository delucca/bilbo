---
id: 01KPMJ3KM05DDQ51FAXQ3DC42G
created: 2026-04-20T01:24-03:00
---

# Especificação do entry-store-schema: limite da coluna Body

Esta nota substitui a nota anterior sobre "entry store body column". O valor novo é este: a coluna Body do entry-store-schema pode guardar no máximo 8 MB por entrada. O limite antigo era de 4 MB e não vale mais. Quem ainda tiver código, teste ou documento citando o valor antigo deve tratá-lo como defasado.

O resto da nota explica o que o limite cobre, por que mudou, o que quem escreve e quem lê precisa respeitar e o que ainda está em aberto. Está escrita com pressa, então vai direto ao ponto.

## Regra atual

A coluna Body do entry-store-schema guarda o conteúdo principal de uma entrada do caderno eletrônico. O teto é de 8 MB por entrada. O limite vale por entrada, não por usuário, por projeto nem por lote de sincronização. Duas entradas diferentes têm, cada uma, o seu próprio teto de 8 MB.

Um ponto que gera dúvida: o limite se aplica ao conteúdo que fica na coluna Body, e não ao tamanho de tudo o que a entrada referencia. Arquivos brutos de instrumento, imagens grandes e anexos pesados não devem morar na coluna. Eles vão para o Azure Blob Storage, e a entrada guarda apenas a referência. Se alguém tentar empurrar um anexo inteiro para dentro de Body só porque agora cabe mais, está usando o campo do jeito errado, mesmo que ainda caiba no teto.

O teto é um máximo, não um tamanho esperado. A maioria das entradas continua muito menor que isso. O aumento existe para os casos de borda, não para mudar o perfil típico.

## Por que o limite subiu

O limite anterior, de 4 MB, estava batendo em entradas legítimas. Os casos que apareceram vinham de saídas de instrumento já convertidas em texto estruturado e anexadas ao corpo da entrada, além de entradas longas com muitas tabelas de resultados coladas. O cientista não tinha como dividir a entrada sem quebrar a leitura do registro, e o compliance não queria entradas fragmentadas, porque cada fragmento a mais é mais uma trilha de auditoria para conferir.

A decisão foi subir o teto para 8 MB em vez de obrigar a divisão. A razão principal é preservar a integridade do registro: uma observação contínua deve continuar sendo uma única entrada, com um único histórico de alterações.

## O que muda para quem escreve na coluna

O serviço de sincronização que grava entradas precisa validar o tamanho de Body contra o novo teto de 8 MB antes de tentar persistir. A validação deve acontecer cedo, na borda, e não depender de o SQL Server recusar a gravação. Um erro vindo do banco por tamanho é difícil de ler para o usuário e pode deixar a mensagem na fila em estado confuso.

Pontos práticos:

- O código em C# que monta a entrada deve ter uma única constante para o limite, usada tanto na validação quanto nas mensagens de erro. Não espalhar o valor por vários arquivos.
- A mensagem de erro para o usuário deve dizer que a entrada passou do máximo permitido e sugerir mover anexos grandes para o armazenamento de blobs.
- Entradas que passam do teto devem ser rejeitadas por inteiro. Nada de truncar em silêncio. Truncar o corpo de um registro de laboratório é pior do que recusar, porque corrompe o conteúdo sem avisar ninguém.
- A rejeição precisa ficar registrada na trilha de auditoria, com quem tentou, quando e em qual entrada, sem copiar o conteúdo recusado para o log.

## O que muda para quem lê e transporta

Do lado do RabbitMQ, as mensagens que carregam o corpo de uma entrada ficam maiores no pior caso. É preciso conferir se a configuração do broker, dos produtores e dos consumidores aceita mensagens desse porte. Se alguma peça no caminho tiver um limite de mensagem menor que o novo teto, a entrada vai passar na validação do esquema e falhar no transporte, o que é o pior tipo de falha porque aparece longe da causa.

A recomendação é que a mensagem leve só o necessário. Quando o corpo for grande, vale considerar enviar uma referência e deixar o consumidor buscar o conteúdo, em vez de empurrar tudo pela fila. Isso não está decidido para todos os fluxos; ver a seção de pontos em aberto.

Os consumidores que leem Body devem evitar carregar tudo em memória de uma vez quando der para processar em fluxo. Com entradas maiores, leituras ingênuas em lote podem pressionar a memória do processo, principalmente quando vários consumidores trabalham em paralelo.

## Efeitos no banco e na migração

A alteração do limite mexe no contrato do entry-store-schema. Se a coluna Body já tinha capacidade de armazenamento acima do teto antigo e o limite de 4 MB era apenas uma regra de validação, a mudança é só de regra e de constante. Se o tipo da coluna ou alguma restrição no SQL Server impunha o limite antigo, é preciso uma migração de esquema. Convém confirmar qual dos dois casos é o real antes de publicar, olhando a definição atual da coluna e as restrições associadas.

Cuidados com a migração, se houver:

- Aplicar primeiro a mudança no banco e só depois liberar a validação nova no serviço. Na ordem inversa, o serviço aceitaria entradas que o banco ainda recusa.
- Entradas já gravadas não mudam. Nenhum dado existente precisa ser reescrito por causa desta alteração.
- Reverter para o limite antigo seria perigoso depois que existirem entradas acima dele. Se um dia for preciso voltar atrás, essas entradas precisarão de tratamento explícito, e não de uma simples troca da constante.

Índices, estatísticas e planos de consulta não deveriam ser afetados, porque o corpo grande não costuma entrar em índice. Mesmo assim, vale observar o tamanho das páginas e o custo das leituras de entradas grandes depois da liberação.

## Auditoria e conformidade

O sistema existe para garantir trilha de auditoria, então a mudança do teto também é uma mudança de regra que precisa ficar rastreável. Esta nota é parte desse rastro: registra que o limite passou de 4 MB para 8 MB e por quê. Quem cuida de compliance deve ser avisado de que entradas maiores agora são possíveis, porque isso afeta revisões, exportações e o tempo de conferência de uma entrada.

Duas garantias não podem se perder com o aumento. Primeira, cada alteração de uma entrada grande continua gerando seu registro de versão, sem exceção por tamanho. Segunda, qualquer verificação de integridade do conteúdo, como somas de verificação, deve continuar cobrindo o corpo inteiro e não só um trecho. Se o cálculo dessas verificações ficou lento com entradas maiores, a solução é otimizar o cálculo, nunca reduzir a cobertura.

## Pontos em aberto e como validar

Ainda não está fechado se todos os fluxos de transporte devem passar a enviar referência em vez do corpo quando a entrada for grande. Quem mexer no consumidor da fila deve decidir isso junto com a equipe responsável pelo broker e registrar a decisão em nota própria, em vez de enterrá-la aqui.

Também falta confirmar se alguma ferramenta de exportação ou de relatório assume o teto antigo em algum buffer. Vale uma busca no código por usos do valor anterior e por suposições de tamanho nos formatadores.

Para validar a mudança, os testes mínimos são:

- Uma entrada exatamente no teto de 8 MB deve ser aceita, sincronizada e lida de volta sem perda.
- Uma entrada um pouco acima do teto deve ser rejeitada por inteiro, com mensagem clara e registro na auditoria.
- Uma entrada entre o limite antigo e o novo, que antes falhava, deve agora passar de ponta a ponta, incluindo o trânsito pela fila.
- Uma entrada pequena deve continuar se comportando como antes, para garantir que nada regrediu.

Se algum desses testes falhar, o problema provavelmente está em outro ponto do caminho que ainda assume o limite de 4 MB, e não no entry-store-schema em si. Comece procurando por configurações de tamanho de mensagem e por constantes duplicadas.
