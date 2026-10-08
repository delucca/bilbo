---
id: 01KMYHXCGH5G1652SKSKQHX2R1
created: 2026-03-30T02:01-03:00
---

# Design do instrument-ingest-worker

O instrument-ingest-worker é o serviço do LabNotebook Sync que pega a saída bruta dos instrumentos de laboratório e a transforma em dados estruturados que o resto do sistema consegue relacionar com as entradas do caderno eletrônico. Ele consome a fila `instrument.raw.v1` do RabbitMQ e interpreta os arquivos CSV dos instrumentos com `CsvHelper 33.0.1`. Esta nota descreve o desenho como está hoje e as razões das escolhas, para quem for mexer nele depois não precisar reconstruir tudo de novo. Foi escrita com pressa, então vai direto ao ponto e algumas coisas estão marcadas como dúvida.

O serviço é escrito em C# sobre .NET, roda como worker em segundo plano, sem interface HTTP própria além do que o orquestrador precisa para saber se está vivo. Ele usa SQL Server para guardar o estado de cada arquivo recebido e as linhas já interpretadas, e Azure Blob Storage para guardar o arquivo original, byte a byte, exatamente como chegou. A regra que guia quase tudo: o arquivo bruto nunca é alterado, e qualquer coisa derivada dele precisa apontar de volta para ele. Quem usa o sistema são cientistas de pesquisa, que querem ver o resultado do instrumento ao lado da entrada do caderno, e responsáveis por conformidade, que querem provar de onde cada número veio.

## Escopo e responsabilidades

O que o instrument-ingest-worker faz: receber a mensagem que anuncia um arquivo novo de instrumento, buscar o conteúdo, validar, interpretar as linhas, gravar o resultado e emitir os eventos de auditoria. O que ele não faz: não decide a qual entrada de caderno um resultado pertence (isso é de outro componente, que faz a associação), não edita dados e não apaga nada. Se alguém pedir para ele corrigir um valor, a resposta é não; correção é uma nova versão feita por pessoa, com justificativa, em outro lugar do sistema.

A fronteira com o produtor das mensagens é a fila `instrument.raw.v1`. O sufixo de versão no nome é proposital: se o formato da mensagem mudar de modo incompatível, cria-se uma fila nova com outra versão, e o worker passa a consumir as duas por um tempo. Não se muda o significado de campos numa fila existente. Isso já foi discutido e a decisão é firme, porque mensagens antigas ainda podem estar paradas na fila ou em reprocessamento quando uma mudança sai.

A mensagem em si é pequena. Ela não carrega o conteúdo do arquivo; carrega a referência para o objeto no Azure Blob Storage, a identificação do instrumento, o momento em que o instrumento terminou a corrida e um identificador de correlação que atravessa todos os logs e eventos de auditoria. O worker confia na referência, mas não confia no resto sem conferir: o que importa para a auditoria é o que está no blob, não o que a mensagem diz sobre ele.

O consumo é feito com confirmação manual. A mensagem só é confirmada depois que o resultado da interpretação foi gravado no SQL Server e o evento de auditoria correspondente foi registrado. Se o processo cair no meio, a mensagem volta para a fila e o arquivo é tentado de novo. Por isso todo o resto do desenho precisa tolerar entrega repetida, o que é o assunto da seção de idempotência.

## Fluxo de ingestão

O caminho feliz, na ordem em que acontece:

Primeiro, o worker recebe a mensagem da fila `instrument.raw.v1` e registra no SQL Server que o arquivo entrou em processamento, ligando esse registro ao identificador de correlação. Se já existe um registro para aquele arquivo em estado final, o worker confirma a mensagem e não faz mais nada, além de anotar que recebeu uma duplicata. Se existe em estado intermediário, ele assume que uma tentativa anterior morreu e continua de onde faz sentido continuar, o que na prática quase sempre significa recomeçar a interpretação, já que ela é barata comparada com a leitura do blob.

Segundo, ele baixa o conteúdo do Azure Blob Storage em fluxo, sem carregar tudo na memória. Os arquivos de alguns instrumentos são grandes, e o worker roda em contêineres com memória limitada. O download alimenta direto o leitor de CSV. Ao mesmo tempo, calcula-se o hash do conteúdo, que é gravado junto com o resultado e entra no evento de auditoria. Esse hash é o elo de prova: quem auditar depois pode baixar o blob de novo e conferir que é o mesmo conteúdo que foi interpretado.

Terceiro, a interpretação. Usamos `CsvHelper 33.0.1` para ler as linhas. A configuração do leitor depende do perfil do instrumento: separador, formato decimal, cultura, linhas de cabeçalho extras, linhas de rodapé com totais. Esses perfis ficam em configuração, um por modelo de instrumento, e não no código das classes de mapeamento. Quando um instrumento novo entra, o trabalho é escrever o perfil e, se o formato for esquisito, um mapeamento específico. O núcleo do worker não deveria mudar.

Quarto, as linhas interpretadas são gravadas no SQL Server em lotes, dentro de uma transação por arquivo. Ou o arquivo inteiro entra, ou nada entra. Foi uma escolha consciente: resultado parcial de uma corrida é pior do que nenhum resultado, porque um cientista pode olhar metade dos pontos e tirar conclusão errada. O custo é que arquivos muito grandes seguram uma transação por mais tempo; até hoje isso não foi problema, mas está na lista de coisas a observar.

Quinto, o worker atualiza o estado do arquivo para concluído, registra o evento de auditoria, publica um evento de domínio dizendo que há dados novos disponíveis para associação e só então confirma a mensagem. A ordem importa. Se publicasse antes de gravar, o consumidor poderia ler algo que ainda não existe.

## Interpretação dos CSV e tratamento de erros

A parte mais trabalhosa do serviço é lidar com a variedade dos arquivos. Instrumentos diferentes, e até versões diferentes de firmware do mesmo instrumento, produzem CSV que só parecem iguais de longe. Os problemas recorrentes são: cabeçalhos com unidades embutidas no nome da coluna, vírgula como separador decimal em alguns equipamentos, datas em formatos locais, campos vazios que significam "não medido" e campos vazios que significam "zero", e linhas de comentário no meio do arquivo. O worker trata cada um desses casos no perfil do instrumento, de forma explícita, e não por adivinhação. Se um valor não bate com o que o perfil espera, é erro, e não uma tentativa de interpretar de outro jeito.

Essa rigidez é intencional por causa da auditoria. Um número que foi "consertado" silenciosamente durante a leitura é um número que ninguém consegue defender depois. Então a política é: interpretou, gravou como veio; não conseguiu interpretar, rejeita o arquivo inteiro e diz exatamente por quê.

Os erros caem em três classes, e o worker se comporta diferente em cada uma.

A primeira classe é erro transitório de infraestrutura: o SQL Server não respondeu, o Azure Blob Storage deu falha de rede, a conexão com o RabbitMQ oscilou. Aqui o worker tenta de novo com espera crescente e um limite de tentativas. Esgotado o limite, a mensagem não é descartada; ela vai para o mecanismo de mensagens mortas, com o motivo anexado, e um alerta é emitido. Não existe cenário em que uma falha de infraestrutura faça o arquivo sumir.

A segunda classe é erro de conteúdo: o CSV está malformado, falta uma coluna obrigatória, um valor não converte, o arquivo está truncado. Repetir não adianta, porque o resultado será o mesmo. O worker marca o arquivo como rejeitado, grava o motivo com a posição aproximada do problema (linha e coluna, quando há), emite o evento de auditoria de rejeição e confirma a mensagem. A rejeição fica visível para quem opera o sistema e para o cientista dono da corrida, que precisa decidir se reexporta o arquivo do instrumento ou abre um chamado. O arquivo bruto continua guardado no blob, mesmo rejeitado, porque a rejeição também é um fato que precisa ser auditável.

A terceira classe é erro de política: o arquivo é válido, mas algo no contexto não fecha, por exemplo um instrumento que não está cadastrado ou um perfil que não existe. Isso é tratado como rejeição com motivo próprio, distinto dos erros de conteúdo, para que os relatórios de operação consigam separar "arquivo ruim" de "cadastro faltando". Quando o cadastro é corrigido, o reprocessamento é manual e deliberado, nunca automático.

Uma observação sobre mensagens de erro: elas são escritas para humanos que não conhecem o código. Dizem qual arquivo, qual instrumento, qual regra foi violada e o que a pessoa pode fazer. Evitamos despejar a exceção crua do `CsvHelper` na tela de quem não é desenvolvedor; a exceção completa vai para o log técnico, ligada ao identificador de correlação.

## Idempotência e trilha de auditoria

Como a entrega do RabbitMQ é pelo menos uma vez, o mesmo arquivo pode chegar duas ou mais vezes. O worker precisa produzir o mesmo resultado final nos dois casos, sem duplicar linhas e sem duplicar eventos de auditoria que signifiquem coisas diferentes. A chave de idempotência é derivada da identidade do arquivo no armazenamento e do hash do conteúdo. Se a identidade é a mesma e o hash também, é duplicata legítima: ignora. Se a identidade é a mesma e o hash difere, isso é grave, porque significa que um objeto no blob mudou depois de anunciado. Nesse caso o worker não processa, rejeita com motivo específico e gera um alerta de severidade alta. O armazenamento deveria impedir isso por imutabilidade, e se aconteceu, alguém precisa investigar.

A trilha de auditoria é o motivo de existir de boa parte do desenho. Cada passo relevante gera um evento: recebimento, início do processamento, conclusão ou rejeição, e a eventual reentrega duplicada. Cada evento carrega o identificador de correlação, o instante, a identificação do arquivo, o hash e a versão do worker que o processou. A versão do worker importa porque, se um bug de interpretação for descoberto depois, é preciso saber quais arquivos passaram por aquela versão para reavaliá-los.

Os eventos de auditoria são gravados no mesmo banco, na mesma transação do resultado, sempre que o evento descreve algo que mudou no banco. Isso evita o caso em que o dado existe mas a prova de que ele foi gravado não existe, ou o contrário. Eventos que descrevem algo que não gerou mudança, como uma duplicata ignorada, são gravados numa transação própria e curta.

A tabela de auditoria só aceita inserção. Nenhum código do worker faz atualização ou exclusão nela, e as permissões da conta usada pelo serviço no SQL Server refletem isso. Foi um pedido direto do pessoal de conformidade, e é a parte que menos se negocia. Se alguém propuser "limpar" eventos antigos para ganhar espaço, a resposta é arquivar com outra estratégia, não apagar.

Outro ponto: relógio. Os instantes gravados vêm do relógio do servidor de banco ou do worker, e nunca do instrumento. O horário que o instrumento declara é guardado como dado, e pode estar errado, já que relógio de equipamento de bancada costuma estar desregulado. A auditoria registra os dois lados, o horário declarado pelo instrumento e o horário em que o sistema soube do arquivo, sem tentar reconciliá-los.

## Operação, desempenho e observabilidade

O worker escala horizontalmente: várias instâncias consomem a mesma fila, e o RabbitMQ distribui as mensagens. Cada instância limita quantas mensagens não confirmadas mantém ao mesmo tempo, porque o processamento de um arquivo grande pode ser demorado e não queremos que uma instância acumule trabalho que outra poderia fazer. O valor desse limite fica em configuração e foi ajustado por observação, não por cálculo; se o perfil de carga mudar, vale revisitar.

A ordem de processamento não é garantida entre arquivos. O desenho não depende dela: cada arquivo é independente na ingestão. Quando uma ordem importa, por exemplo corridas de calibração antes das corridas de amostra, essa lógica é responsabilidade de quem associa os resultados ao caderno, olhando os instantes declarados, e não do worker.

Sobre desempenho, o gargalo observado não é a leitura do CSV, que é rápida, e sim a gravação no SQL Server. Por isso as linhas são enviadas em lotes e não uma a uma. Se a latência do banco subir, o worker desacelera naturalmente, porque não confirma mensagens enquanto não termina; a fila cresce e o alerta de profundidade de fila avisa antes que alguém perceba pelo lado do usuário. A memória é mantida baixa justamente por processar em fluxo, o que também significa que a análise do arquivo não pode depender de olhar o arquivo todo antes de começar. Qualquer validação que exija o arquivo inteiro, como conferir um total no rodapé contra a soma das linhas, é feita ao final e, se falhar, desfaz a transação.

A observabilidade tem três frentes. Logs estruturados, sempre com o identificador de correlação, para seguir um arquivo do recebimento ao fim. Métricas: quantidade de arquivos por resultado (concluído, rejeitado, duplicata), tempo de processamento, profundidade da fila e quantidade de mensagens mortas. E os próprios eventos de auditoria, que servem também como histórico consultável por quem opera. Os alertas que importam são poucos: mensagens mortas aparecendo, hash divergente para a mesma identidade, fila crescendo sem parar e taxa de rejeição fora do normal para um instrumento específico, que costuma indicar atualização de firmware que mudou o formato.

Para reprocessar, há um caminho manual: um operador autorizado marca o arquivo para nova tentativa, e isso gera seu próprio evento de auditoria com o nome de quem pediu e o motivo. O reprocessamento não apaga o resultado anterior; cria uma nova versão e mantém a antiga visível no histórico. Quem consulta vê qual versão está vigente e por quê.

## Decisões em aberto e armadilhas conhecidas

Algumas coisas ainda não estão resolvidas, e vale deixar escrito para ninguém achar que foram esquecidas.

A transação única por arquivo é simples e correta, mas pode ficar incômoda com arquivos de tamanho extremo. A alternativa seria gravar em área de preparação e promover ao final, mantendo a atomicidade do ponto de vista de quem lê sem segurar uma transação longa. Ainda não valeu o esforço. Se aparecer instrumento que gere arquivos muito maiores que os atuais, esta é a primeira decisão a rever.

A política de retenção dos blobs brutos ainda depende de definição com a conformidade. Hoje nada é apagado. O desenho assume que o bruto fica pelo menos tanto quanto o dado derivado, porque o bruto é a prova. Qualquer ciclo de vida automático no armazenamento precisa passar por revisão deles antes de ser ligado.

A atualização do `CsvHelper 33.0.1` para versões futuras deve ser tratada com cuidado. Mudanças de comportamento em conversão de tipos ou em tratamento de campos vazios alteram o resultado da interpretação sem que nenhuma linha do nosso código mude. Por isso a versão da biblioteca é fixada, e atualizar exige rodar a coleção de arquivos de exemplo de cada instrumento e comparar o resultado antes e depois, linha a linha. Como a versão do worker entra na auditoria, a troca de biblioteca deve vir acompanhada de nova versão do worker, para que seja possível saber qual interpretação cada arquivo recebeu.

Armadilhas que já morderam ou quase:

- Cultura do processo. Se o leitor usar a cultura padrão da máquina em vez da cultura definida no perfil, o mesmo arquivo gera números diferentes em ambientes diferentes. A cultura precisa vir sempre do perfil.
- Campos vazios. Tratar vazio como zero é erro silencioso. O perfil diz, coluna por coluna, se vazio é ausente ou inválido.
- Codificação do arquivo. Alguns instrumentos gravam com marca de ordem de bytes, outros não, e alguns usam codificações antigas. Isso também é parte do perfil, e deve ser verificado quando um modelo novo entra.
- Confirmação antecipada. Mover a confirmação da mensagem para antes da gravação deixa o código mais rápido e perde dados em queda de processo. Não fazer.
- Mensagens mortas esquecidas. A fila de mensagens mortas não é um depósito; alguém precisa olhar. Se o alerta for silenciado, o risco é um arquivo de instrumento nunca chegar ao caderno e ninguém notar.
- Duplicata com conteúdo diferente. Já descrito acima; é o único caso em que o worker se recusa a seguir mesmo sendo "só uma repetição".

Por fim, uma lista curta do que conferir antes de mexer neste componente: o contrato da mensagem em `instrument.raw.v1` não pode mudar de significado; o arquivo bruto não pode ser alterado nem apagado; o resultado de um arquivo é tudo ou nada; cada passo relevante precisa deixar evento de auditoria com identificador de correlação; e qualquer mudança na interpretação precisa ser rastreável pela versão do worker. Se uma alteração proposta quebra qualquer uma dessas cinco coisas, ela precisa de conversa com a conformidade antes de virar código.

Também vale lembrar que o worker é o ponto onde a confiança do sistema começa. Tudo que vem depois, associação, revisão, assinatura de entradas, relatórios, assume que o que foi gravado aqui reflete fielmente o que o instrumento entregou. Por isso é preferível um serviço que rejeita com clareza a um serviço que aceita e conserta em silêncio. Quando houver dúvida entre ser tolerante e ser rastreável, escolher rastreável.

Pendências práticas para a próxima pessoa que pegar isto: documentar o conjunto de perfis de instrumento num lugar único e legível por cientistas, para que eles consigam dizer se um formato novo está coberto; escrever testes de regressão com arquivos reais anonimizados de cada modelo suportado; e revisar o limite de mensagens simultâneas por instância depois do próximo aumento de volume. Nenhuma dessas pendências muda o desenho, mas todas reduzem a chance de surpresa em produção.
