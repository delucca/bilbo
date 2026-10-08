---
id: 01KAFXW2JDM8RBVS48E411Q2NZ
created: 2025-11-20T03:05-03:00
---

# instrument-ingest-worker: o que vigiar ao mexer

Notas rápidas sobre o que costuma dar errado quando alguém mexe no instrument-ingest-worker. Não é um manual nem uma lista de decisões. É o que vale reler antes de abrir um PR neste componente. O worker fica entre os instrumentos de laboratório e o resto do LabNotebook Sync: pega a saída bruta dos equipamentos, valida, guarda o arquivo original, grava os metadados e entrega tudo para quem vincula o resultado à entrada do caderno eletrônico. Por causa do requisito de trilha de auditoria, um erro aqui quase nunca é só um bug. Pode virar um problema de conformidade que só aparece meses depois, quando um auditor pede para reconstruir o que aconteceu com uma amostra.

A regra geral é simples: se a mudança altera o que é gravado, quando é gravado ou em que ordem, trate como mudança de comportamento auditável, mesmo que o diff pareça pequeno. Refatoração "limpa" que muda a ordem de duas gravações já causou dor em projetos parecidos.

## Mensagens do RabbitMQ e semântica de entrega

O worker consome mensagens e precisa assumir que a mesma mensagem pode chegar mais de uma vez. Isso não é caso raro: acontece em reinício do processo, em perda de conexão antes do ack, em redistribuição depois de timeout. Qualquer alteração no fluxo de consumo tem que manter o processamento idempotente. Se você adicionar um passo novo, pergunte o que acontece se ele rodar de novo com a mesma entrada, depois de um sucesso parcial.

O ponto do ack importa muito. Confirmar cedo demais perde dados quando o processo cai no meio. Confirmar tarde demais gera duplicatas e reentregas em cascata. Ao mudar a ordem entre gravar no banco, gravar no Blob e confirmar a mensagem, desenhe no papel o que sobra em cada ponto de falha. Não confie só no caminho feliz dos testes.

Cuidado com mensagens venenosas. Uma mensagem que sempre falha e volta para a fila para sempre trava o consumo ou enche o log. Quem mexer no tratamento de erro precisa conferir se o destino de mensagens rejeitadas continua funcionando e se alguém olha para ele. Mensagem rejeitada de instrumento é dado científico que não entrou no sistema, e o cientista pode nem saber.

A ordem das mensagens não é garantida em todos os cenários, principalmente com mais de um consumidor ativo e com reentrega. Código que assume que a leitura final de uma corrida chega depois das leituras parciais vai quebrar de vez em quando e de um jeito difícil de reproduzir. Se você precisa de ordem, ela tem que vir do conteúdo da mensagem (sequência, marca de tempo do instrumento) e não da chegada.

Mudanças em filas, exchanges, chaves de roteamento ou argumentos de declaração precisam ser combinadas com o ambiente. Redeclarar uma fila com argumentos diferentes dos que já existem no broker costuma falhar na subida, e às vezes só em produção, onde a fila antiga ainda está lá. Não renomeie nada achando que é só código.

Prefetch e concorrência também mexem no comportamento sem aviso. Aumentar o paralelismo acelera a ingestão, mas expõe corridas que antes não apareciam, em especial na gravação de metadados e na associação ao caderno. Mude isso com cuidado e olhe o efeito nas travas do banco.

## Contrato do payload e versões de formato

O formato da mensagem é um contrato com quem publica, que pode ser um agente instalado perto do instrumento ou outro serviço. Publicadores e consumidores são atualizados em momentos diferentes. Então o worker tem que tolerar campos que não conhece e, por um tempo, a ausência de campos novos. Não transforme campo opcional em obrigatório sem uma janela de transição combinada.

Ao adicionar um campo, defina o que significa quando ele falta. Ao remover ou renomear, pense nas mensagens que já estão na fila ou no destino de rejeitadas e que ainda carregam o formato velho. Reprocessar uma fila antiga depois de um deploy novo é um cenário real, e o código de leitura precisa continuar entendendo o que foi publicado antes.

A serialização merece atenção. Mudanças de configuração do serializador, como política de nomes, tratamento de nulos, enums como texto ou como número e formato de datas, alteram o que passa pela rede sem mudar nenhuma classe. Um enum novo que o publicador já manda e o worker ainda não conhece não pode derrubar o lote inteiro. Decida de propósito se vira um valor desconhecido tratado com cuidado ou uma rejeição explícita e registrada.

Fuso horário e relógio do instrumento são fonte clássica de confusão. Instrumentos têm relógios próprios, às vezes errados, às vezes sem fuso. O worker deve guardar o que o instrumento informou como veio e também o momento em que a ingestão aconteceu, sem misturar os dois. Qualquer "normalização" que sobrescreva o valor original destrói evidência.

Unidades e precisão numérica seguem a mesma lógica. Converter unidade, arredondar ou trocar o tipo numérico (por exemplo, de decimal para ponto flutuante) pode alterar resultados científicos de forma silenciosa. Se uma mudança toca em tipos numéricos, trate como mudança de dado e peça revisão de quem entende do domínio.

## Arquivos brutos no Azure Blob Storage

O arquivo bruto do instrumento é o registro primário. A regra de ouro: o original nunca é alterado depois de gravado. Qualquer conversão, extração ou normalização gera um artefato derivado separado, com ligação clara ao original. Se uma mudança no worker sobrescreve um blob existente, mesmo "só para corrigir", está violando o princípio de que o auditor consegue ver exatamente o que o equipamento produziu.

Pense em integridade. O worker deve calcular e guardar um resumo criptográfico do conteúdo e usá-lo para detectar corrupção e duplicata. Mudar o algoritmo ou o momento do cálculo (antes ou depois de decodificar, antes ou depois de qualquer transformação de fim de linha ou codificação) invalida a comparação com tudo que já foi gravado. Se precisar mudar, os dois valores têm que coexistir por um tempo.

Nomes e organização dos blobs são quase um contrato. Outras partes do sistema, relatórios e exportações para auditoria podem depender da estrutura de contêineres e prefixos. Alterar a convenção de nomes sem migrar o que existe quebra links antigos. Evite colocar no nome do blob dado que possa mudar depois, como o nome do projeto ou do usuário.

Observe a política de retenção e imutabilidade configurada na conta de armazenamento. Código que tenta apagar ou regravar pode falhar justamente porque a conta está configurada para proibir isso, e isso é desejado. Não contorne com gambiarra. Também não assuma que um teste local com emulador reproduz essas regras; ele costuma ser mais permissivo.

Uploads grandes exigem cuidado com timeouts, reenvio e blocos parciais. Um upload interrompido pode deixar um blob incompleto que parece existir. Antes de considerar a gravação concluída, confirme que o commit do conteúdo terminou. Ao fazer retry, confirme que não está criando um segundo blob órfão ou, pior, ligando o registro no banco a um blob truncado.

Por fim, vazamento de memória e uso de disco: ler arquivo inteiro na memória é tentador e funciona nos testes pequenos. Instrumentos reais produzem arquivos grandes, e vários ao mesmo tempo. Prefira streaming e confira que os fluxos são liberados também nos caminhos de erro.

## SQL Server: transações, travas e esquema

A gravação no SQL Server e a gravação no Blob não participam da mesma transação. Isso é a fonte de quase todas as inconsistências possíveis neste componente. O código precisa de uma ordem definida e de um jeito de se recuperar quando só uma das duas metades aconteceu. Quem altera essa sequência está mexendo no coração do worker. Releia o raciocínio de recuperação antes de mudar e deixe registrado no PR qual é o estado intermediário aceito.

Migrações de esquema têm que ser compatíveis com a versão anterior do worker rodando ao mesmo tempo, porque deploys não são instantâneos e pode haver instâncias velhas consumindo. O padrão seguro é adicionar primeiro, migrar o uso depois e só então remover. Remover coluna ou apertar uma restrição no mesmo release que muda o código é pedir incidente.

Tabelas de auditoria são só de acréscimo. Não escreva código que faça update ou delete nelas, nem "limpezas" de dados de teste que apontem para o banco errado. Se um índice ou uma partição novos forem necessários, avalie o efeito em tabelas grandes: criar índice sem pensar em bloqueio pode parar a ingestão enquanto o banco trabalha.

Deadlocks e esperas por trava aparecem quando o paralelismo aumenta ou quando a ordem de acesso às tabelas muda. Mantenha uma ordem consistente de acesso e transações curtas. Não faça chamadas de rede (Blob, HTTP, fila) dentro de uma transação de banco aberta; isso segura travas por tempo imprevisível.

O nível de isolamento e as opções de conexão afetam o resultado. Leituras sujas ou instantâneos desatualizados podem fazer o worker decidir duplicata ou novidade com base em informação velha. Chaves únicas no banco são a defesa final contra duplicata; não substitua por verificação só em código.

Sobre o mapeamento do ORM: mudanças de versão da biblioteca ou de convenção podem gerar consultas diferentes, incluindo carregamento tardio que vira uma enxurrada de consultas pequenas. Olhe o SQL gerado dos caminhos quentes depois de atualizar dependências. Tipos de data e hora, precisão e colação de texto também podem mudar o comportamento de comparação.

## Trilha de auditoria e conformidade

Este worker produz evidência. Cada ingestão precisa deixar um registro de quem ou o que enviou, de qual instrumento veio, o que foi recebido, o que foi feito com o dado e com que resultado, inclusive quando falhou ou foi rejeitado. Rejeições sem registro são tão ruins quanto perdas de dado, porque o compliance precisa explicar por que algo não está no caderno.

Não troque um registro de auditoria por um log comum. Logs de aplicação rodam, rotacionam e podem ser descartados; a trilha de auditoria é dado de negócio com retenção própria. Se uma mudança "limpa" logs, confira que ela não está tocando na trilha. E o contrário: não despeje dado sensível nos logs comuns só para facilitar depuração.

Identidade e atribuição: o registro precisa dizer qual identidade técnica ou humana originou o dado. Ao mexer em autenticação entre agentes e worker, não deixe que ingestões passem a ser atribuídas a uma conta genérica. Atribuição perdida não se recupera depois.

Relógio e ordem dos eventos na trilha devem vir de uma fonte confiável do servidor, não do cliente. Se o código passa a aceitar a hora enviada pelo publicador como hora do evento de auditoria, isso abre espaço para registro retroativo. Guarde ambas, e deixe claro qual é qual.

Correções são novos registros. Se um dado ingerido estava errado, o fluxo correto é registrar a correção apontando para o original, e não editar. Qualquer ferramenta de reprocessamento que você escrever tem que seguir isso: reprocessar gera nova versão rastreável, nunca troca a antiga em silêncio.

Se a mudança afeta algo que um auditor veria (campos gravados, formato dos registros, retenção, ordem), avise quem cuida de conformidade antes de publicar. Isso não é burocracia: eles podem ter procedimentos validados que dependem do comportamento atual.

## Falhas parciais, retries e recuperação

O worker vai falhar no meio de algo, com certeza. Rede cai, o Blob responde devagar, o banco entra em failover, o broker reinicia. O código deve distinguir falha transitória de falha permanente. Retentar uma falha permanente só desperdiça recurso e atrasa o resto; desistir cedo de uma transitória joga dado fora sem necessidade.

Retries precisam de espera crescente com variação aleatória, e de um limite. Sem isso, uma queda do banco vira uma tempestade de tentativas simultâneas quando ele volta. Cuidado também com camadas de retry empilhadas: a biblioteca de cliente tenta, o seu código tenta, a fila reentrega. O efeito multiplicado pode ser enorme e confuso nos logs.

Cada etapa precisa ser retomável. Se a gravação do arquivo funcionou e a do metadado não, a próxima tentativa deve reconhecer o que já existe e continuar, não duplicar nem falhar por conflito. Isso depende de identificadores determinísticos para cada ingestão, derivados do conteúdo ou da origem, e não gerados do zero a cada tentativa.

O desligamento do processo merece teste explícito. Em deploy ou escala para baixo, o worker recebe sinal de parada. Ele deve parar de pegar mensagens novas, terminar ou devolver o que está em andamento e sair dentro do tempo que a plataforma dá. Tarefas em segundo plano que ignoram o token de cancelamento deixam trabalho pela metade e mensagens sem ack.

Tenha cuidado com exceções engolidas. Um bloco que captura tudo e segue em frente é muito tentador em um worker de longa duração, mas esconde falhas que deveriam virar rejeição registrada ou alerta. Por outro lado, deixar uma exceção inesperada derrubar o processo inteiro por causa de uma mensagem ruim também é ruim. O equilíbrio é tratar por mensagem, registrar com contexto e seguir.

Por último, a fila de rejeitadas precisa de um dono. Se ninguém olha, ela vira cemitério de dados de experimentos. Mudanças que aumentam o volume de rejeições (validação mais estrita, por exemplo) devem vir com aviso a quem opera.

## Instrumentos, formatos e validação

Cada fabricante e modelo de instrumento tem as suas esquisitices: codificação de caracteres, separador decimal, finais de linha, cabeçalhos opcionais, campos que mudam de posição conforme o firmware, arquivos que são escritos aos poucos enquanto a corrida ainda acontece. Um parser que funciona com os exemplos do repositório pode quebrar com a saída real de uma unidade com firmware diferente. Antes de apertar uma validação, olhe amostras reais e variadas, anonimizadas, e não só as de teste.

Validar demais rejeita dado bom; validar de menos deixa lixo entrar no caderno. Não há regra única. A decisão é de domínio e precisa de quem usa o instrumento. Uma mudança de validação deve ser comunicada como mudança de comportamento, porque o cientista vai ver arquivos que antes entravam e agora não entram, ou o inverso.

Arquivos incompletos são um problema recorrente. O instrumento ou o agente pode publicar enquanto o arquivo ainda está sendo escrito. O worker precisa de algum critério para saber que o arquivo está completo, e esse critério não deve se basear apenas em tempo de espera. Se alterar essa lógica, teste com arquivos que crescem.

Codificação e localidade: nunca dependa da cultura padrão da máquina para interpretar números e datas. O que roda bem no ambiente de desenvolvimento pode ler vírgula como separador de milhar em outro servidor. Use sempre cultura invariável ou explícita, escolhida por formato de instrumento.

Ao adicionar suporte a um instrumento novo, isole o parser dos outros. Evite ajustar um parser genérico compartilhado de um jeito que muda a leitura de formatos antigos. Tenha amostras de regressão de cada formato já suportado e rode todas antes de fundir a mudança.

Dados sensíveis podem aparecer em nomes de amostra, identificadores de paciente ou de participante em alguns tipos de estudo. O worker não deve copiar esse conteúdo para lugares que não estavam previstos, como mensagens de erro, nomes de blob ou telemetria.

## Configuração, segredos e ambientes

Muita coisa que parece detalhe de configuração muda o comportamento de verdade: tempo limite, tamanho de lote, paralelismo, nomes de filas, contêineres, políticas de retry. Mudou o padrão no código? Confira o que está sobrescrito em cada ambiente, porque o valor que vale pode ser outro. E o contrário: uma opção removida do código mas ainda presente na configuração do ambiente costuma ser ignorada em silêncio.

Segredos, como cadeias de conexão do banco, do broker e do armazenamento, não entram no repositório, nem em testes, nem em logs. Prefira identidade gerenciada quando a plataforma suportar, e confira as permissões mínimas necessárias. Um PR que pede mais permissão do que antes merece pergunta sobre por quê.

Ambientes de teste e produção devem ser claramente separados. Já houve projetos em que um teste de integração apontou, por configuração herdada, para recursos reais. Se você roda algo que escreve, confirme duas vezes qual banco, qual broker e qual conta de armazenamento estão ligados antes de começar. Ainda mais por causa da natureza só de acréscimo da trilha: sujeira em produção não se limpa.

Feature flags ajudam em mudanças arriscadas, mas flag esquecida vira comportamento oculto. Se criar uma, anote quem a remove e quando, e garanta que os dois caminhos continuam testados. Cuidado ainda com flags que mudam o que é gravado: dois ambientes passam a produzir registros de formatos diferentes.

Atualizações de dependências, tanto do .NET quanto de bibliotecas de cliente do RabbitMQ, do SQL Server e do Azure, são mudanças de comportamento potenciais. Leia as notas de versão sobre padrões que mudaram (criptografia de conexão, validação de certificado, tempos limite, política de reconexão). Atualizar só porque há um aviso do scanner, sem rodar a bateria de integração completa, é arriscado neste componente.

## Testes, observabilidade e operação

Testes unitários dos parsers e das regras de validação são baratos e devem ser abundantes. Mas os problemas mais caros deste worker estão nas fronteiras: mensagem duplicada, falha entre o Blob e o banco, reinício no meio, ordem trocada. Esses cenários só aparecem com testes de integração que realmente injetam falhas. Se sua mudança toca no fluxo de gravação ou de ack, adicione ou execute testes assim, e diga no PR o que foi coberto.

Emuladores e contêineres locais ajudam, mas divergem do serviço real em pontos importantes, como limites, consistência, regras de imutabilidade e mensagens de erro. Não conclua que está tudo certo só porque passou localmente. Quando possível, valide em um ambiente de homologação com infraestrutura parecida com a real.

Observabilidade: mantenha identificadores de correlação por ingestão desde a mensagem até o registro final, para dar para seguir um arquivo de ponta a ponta. Ao refatorar, é fácil perder o contexto nos logs e nas métricas. Métricas úteis incluem atraso da fila, taxa de rejeição, tempo por etapa e quantidade de reentregas. Se alguma delas some depois da sua mudança, quem opera vai ficar às cegas justo quando algo der errado.

Alertas devem refletir risco real: ingestão parada, fila de rejeitadas crescendo, divergência entre o que está no armazenamento e o que está no banco. Ao mudar nomes de métricas ou mensagens de log, verifique se algum painel ou alerta depende deles.

Plano de reversão: antes de publicar, saiba como voltar. Reverter código é fácil; reverter dados gravados em formato novo não é. Se a versão nova grava algo que a anterior não entende, a volta fica difícil. Prefira mudanças aditivas e com compatibilidade nos dois sentidos durante a transição.

Por fim, comunique. Pesquisadores e compliance dependem de comportamento previsível. Se a mudança altera o que chega ao caderno, o tempo que leva ou o que é rejeitado, avise antes e não depois. E se descobrir uma armadilha nova mexendo aqui, volte e atualize esta nota.

## Lista curta para revisar antes de fundir

Isto é só um lembrete para a hora de revisar, sem ordem de importância.

A mudança continua idempotente diante de reentrega? A ordem entre gravar o arquivo, gravar o metadado e confirmar a mensagem foi preservada ou conscientemente alterada, com o estado intermediário descrito? O arquivo original continua intocado e os derivados continuam separados e ligados a ele? A trilha de auditoria continua só de acréscimo, com atribuição e hora do servidor? Versões antigas de mensagens e do worker ainda funcionam durante o deploy? Rejeições continuam registradas e alguém as vê? Parsers foram testados com amostras reais de mais de um firmware? Configuração e segredos foram conferidos em cada ambiente? Existe caminho de reversão que não perde nem corrompe dado? Quem opera e quem cuida de conformidade foram avisados do que muda?

Se alguma resposta for "não sei", pare e descubra antes de fundir. Neste componente, o custo de descobrir depois é quase sempre maior, e a parte difícil é que o erro pode ficar escondido por muito tempo até alguém precisar daquele registro.
