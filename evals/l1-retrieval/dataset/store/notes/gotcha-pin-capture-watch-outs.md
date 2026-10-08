---
id: 01K8AHVF74CDH2RDR4CDKV8A38
created: 2025-10-24T04:27-03:00
---

# pin-capture-app: pontos de atenção ao mexer no componente

Anotação geral sobre o que costuma dar problema quando alguém altera o pin-capture-app. Não é uma lista de decisões nem de requisitos; é um lembrete de armadilhas que se repetem. O pin-capture-app roda no celular do motorista, quase sempre em rua, com rede ruim, bateria no limite e pressa. Qualquer mudança que funcione bem no escritório, com Wi-Fi e aparelho novo, pode quebrar na ponta. Antes de mexer, vale lembrar que o que o motorista captura é prova de entrega, e perder uma prova é pior do que atrasar o envio dela. Quando houver dúvida entre ser rápido e ser seguro com os dados já capturados, escolha a segurança dos dados.

Outra coisa de partida: o componente conversa com o backend por contratos em Protocol Buffers e com o Firestore por regras e índices que vivem fora do aplicativo. Muita mudança aparentemente local do pin-capture-app tem efeito em outro lugar. Se você só olhou o código Kotlin, provavelmente ainda não olhou tudo.

## Offline primeiro, sempre

O modo offline não é um recurso extra, é o modo normal de operação. Todo fluxo novo precisa funcionar sem rede do começo ao fim: abrir a rota, ver as paradas, capturar a prova, marcar a parada como concluída. Se uma tela nova depende de uma chamada de rede para renderizar, ela vai travar na mão do motorista no pior momento.

Cuidado com código que assume que uma leitura do Firestore volta rápido. Com a persistência local ligada, a leitura pode vir do cache, e o cache pode estar velho. Não trate dado vindo do cache como se fosse confirmado pelo servidor. Distinga, na interface e na lógica, o que está apenas salvo no aparelho do que já foi aceito pelo backend. Um motorista que vê "concluído" e depois descobre que nada chegou perde a confiança no app inteiro.

Também não confie em callbacks de sucesso de escrita no Firestore como sinal de que o servidor recebeu. Com o aparelho offline, essa confirmação pode demorar indefinidamente ou nunca chegar na sessão atual. Se a lógica de negócio espera por ela para liberar a próxima parada, o fluxo para.

Por fim, teste sempre os casos de borda de conectividade: rede que cai no meio do envio, rede que volta e cai de novo, rede presente mas sem saída para a internet (portal cativo, sinal fraco que conecta e não trafega). Modo avião ligado e desligado não cobre isso. Esses casos intermediários é que geram duplicidade e estado preso.

## Fila de envio e idempotência

O envio das provas passa por uma fila local que tenta de novo até dar certo. Isso significa que a mesma prova pode ser enviada mais de uma vez: o app morreu depois de enviar e antes de registrar o sucesso, a resposta se perdeu, o sistema reiniciou o trabalho em segundo plano. Toda mudança nessa área precisa preservar a idempotência. O backend e o app devem tratar o reenvio da mesma prova como a mesma prova, nunca como uma nova.

A identidade da prova tem que ser gerada no aparelho, no momento da captura, e permanecer estável por toda a vida do item na fila. Se alguém trocar a forma de gerar esse identificador, ou regenerá-lo no reenvio, aparecem entregas duplicadas no painel e, pior, provas que parecem faltar porque foram gravadas sob outro identificador.

Observe a ordem dos itens. Em algumas situações a ordem de envio importa, por exemplo quando uma correção de uma prova depende da prova original já ter chegado. Mudar a fila para paralelizar mais pode quebrar essa suposição sem nenhum teste vermelho.

Tenha cuidado com política de nova tentativa. Intervalos curtos demais drenam bateria e dados do motorista; longos demais deixam a prova parada quando a rede volta. E erros permanentes (dado inválido, rejeição de permissão) não devem ser repetidos para sempre: o item precisa sair do ciclo de tentativas e ficar visível para alguém tratar. Item que falha em silêncio e permanece na fila é um problema que só aparece semanas depois, numa disputa com o cliente.

## Captura de foto e assinatura

A captura é o coração do componente. Qualquer regressão aqui é crítica. Pontos recorrentes: a câmera do sistema e a câmera embutida se comportam diferente conforme o fabricante; a orientação da imagem muitas vezes vem num metadado e não nos pixels; e a atividade pode ser destruída enquanto o app de câmera está em primeiro plano, porque o sistema precisa de memória. Se o estado da captura em andamento não for salvo, o motorista volta e encontra a tela vazia.

A imagem precisa ser gravada em armazenamento do app de forma durável antes de qualquer outra coisa. Só depois entram compressão, redimensionamento e enfileiramento. Se a ordem for invertida e o processo morrer no meio, a prova se perde sem rastro.

Compressão é um equilíbrio. Reduzir demais prejudica o valor da prova (um rosto, uma fachada, uma etiqueta ilegível); reduzir de menos estoura dados móveis e armazenamento. Não mude os parâmetros sem olhar amostras reais de fotos tiradas em baixa luz, que é o caso comum no fim do dia.

A assinatura tem sua própria sutileza: o traço precisa ser capturado de forma independente da densidade de tela, senão a mesma assinatura sai com aparência diferente em aparelhos diferentes. Cuide também de limpar e redesenhar corretamente após rotação de tela ou troca de aplicativo, e de impedir que um toque acidental envie uma assinatura vazia.

Por fim, metadados da captura (momento, posição, quem capturou) fazem parte da prova. Não os recalcule depois, no envio; registre no instante da captura.

## Localização e GPS

A posição anexada à prova é um dos argumentos mais fortes em caso de contestação. Por isso, mudanças na leitura de localização exigem cuidado redobrado. O sistema pode devolver uma posição antiga guardada em cache, com baixa precisão, e o código pode aceitá-la como se fosse atual. Sempre considere a idade e a precisão da leitura, e decida de forma explícita o que fazer quando ela é ruim: pedir nova leitura, avisar o motorista ou registrar a qualidade junto com o ponto.

Permissões de localização mudam entre versões do Android, e o usuário pode revogá-las a qualquer momento, inclusive com o app aberto. A permissão de uso em segundo plano é tratada à parte e costuma ser negada. Não escreva fluxos que presumam que a permissão obtida na primeira abertura continua valendo.

Também há o caso de posição simulada. Alguns aparelhos permitem aplicativos de localização falsa; se o produto se importa com isso, a detecção deve ficar registrada nos dados, mesmo que não bloqueie nada. Não remova essa informação por achar que é ruído.

Outra armadilha é o consumo de bateria: pedir localização de alta precisão de forma contínua esgota o aparelho antes do fim da rota. Prefira pedir a leitura quando for necessária e pare de ouvir quando sair da tela. Se mudar a frequência de atualizações, teste num aparelho de verdade rodando por horas, não só no emulador, que se comporta de modo muito diferente.

## Esquema Protocol Buffers e compatibilidade

As mensagens trocadas com o backend são definidas em Protocol Buffers, e há versões antigas do aplicativo instaladas em campo por muito tempo. Motoristas não atualizam na hora, e alguns aparelhos ficam semanas sem rede boa para baixar a atualização. Portanto, qualquer alteração de esquema precisa ser compatível nos dois sentidos: o app novo conversando com o backend antigo e o app antigo conversando com o backend novo.

Regras de ouro: não reutilize o número de um campo removido; não mude o tipo de um campo existente; adicione campos novos como opcionais e trate sua ausência com um valor padrão sensato; reserve nomes e números de campos aposentados. Mudar o significado de um campo mantendo o tipo é a variante mais perigosa, porque nada quebra na compilação e os dados passam a significar outra coisa.

Cuidado com enumerações. Um valor novo que o app antigo não conhece chega como valor desconhecido, e o código Kotlin que faz uma verificação exaustiva pode lançar exceção ou cair num ramo errado. Sempre trate o caso desconhecido de forma deliberada.

O que está na fila local também foi serializado com o esquema da época. Itens guardados antes da atualização do app serão lidos depois dela. Se a leitura da fila depender de campos que não existiam antes, itens antigos vão falhar. Teste atualizando o app com a fila cheia, não apenas com instalação limpa.

Por fim, mantenha a geração de código alinhada entre os projetos que compartilham o esquema; descompasso silencioso entre as cópias é causa clássica de bug que só aparece em produção.

## Firestore: regras, índices e custos de leitura

As regras de segurança do Firestore e os índices ficam fora do código do pin-capture-app, mas o app depende deles o tempo todo. Se você mudar uma consulta (novo filtro, nova ordenação), provavelmente vai precisar de um índice composto, e ele precisa existir no ambiente antes da versão do app que o usa. Do contrário a consulta falha, e falha apenas em quem já atualizou.

A mesma lógica vale para regras. Uma escrita nova em outro caminho de documento pode ser negada pelas regras vigentes, e com a escrita offline isso só aparece depois, quando o aparelho sincroniza e o servidor recusa. O app local já mostrou sucesso. Considere sempre como o app reage a uma rejeição tardia de escrita: o item deve ficar visível como problema, não sumir.

Cuidado com a modelagem de documentos. Documentos grandes, listas que crescem sem limite e atualizações frequentes no mesmo documento por muitos clientes geram contenção e custo. Pense em quantas leituras uma tela nova provoca por abertura; multiplicado por muitos motoristas e muitas aberturas ao dia, vira conta relevante e lentidão.

Listeners em tempo real devem ser desligados quando a tela sai de cena. Escutas esquecidas continuam consumindo leituras e bateria. Também fique atento à resolução de conflitos: quando dois clientes, ou o app e um painel de operações, alteram o mesmo documento, a última escrita vence. Se a regra de negócio exige outra coisa, use transações ou campos separados, sabendo que transações não funcionam offline do mesmo jeito que escritas simples.

Por fim, evite depender do horário do aparelho para ordenar eventos que importam; relógios de celular são imprecisos e às vezes alterados à mão. Use o carimbo do servidor onde a ordem for relevante.

## Autenticação Firebase e sessão do motorista

O motorista se autentica pelo Firebase, e a sessão precisa sobreviver a períodos sem rede. Um token expirado com o aparelho offline não pode derrubar o acesso às paradas do dia nem impedir capturas. Garanta que a falta de renovação de credenciais não bloqueie a operação local; a renovação acontece quando a rede voltar, e o envio atrasado deve usar a credencial renovada.

Por outro lado, saída de sessão é um momento delicado. Se o motorista sai (ou é deslogado pelo sistema) com provas ainda na fila, o que acontece com elas? Limpar a fila no logout destrói provas; mantê-la sem dono pode misturar dados entre motoristas num aparelho compartilhado. Qualquer mudança nesse fluxo precisa responder a essa pergunta de forma explícita e testada.

Aparelhos compartilhados entre turnos existem. Cache local, fotos armazenadas e preferências devem estar associados a quem os criou. Cuidado para que a tela de um motorista não mostre paradas ou dados do anterior depois de uma troca rápida.

Também tenha em mente a revogação de acesso: quando alguém deixa a operação, o acesso deve cessar, mas o que já foi capturado e ainda não enviado continua sendo prova legítima. Esse caso raramente é testado e merece atenção quando mexer em autenticação.

Evite registrar tokens, identificadores de sessão ou dados de credencial em logs, mesmo em builds de depuração que possam vazar para relatórios de erro.

## Ciclo de vida Android e processos em segundo plano

O Android mata processos, destrói atividades e restringe trabalho em segundo plano de maneiras que variam por fabricante e por versão. Em aparelhos de algumas marcas, a economia de bateria é agressiva e interrompe tarefas que em um emulador rodariam sem problema. Não conclua que o envio em segundo plano funciona só porque funcionou no seu aparelho.

Use os mecanismos oficiais do Jetpack para trabalho que precisa terminar mesmo se o app for fechado, e desenhe cada tarefa para ser retomável e idempotente. Trabalho longo preso a uma atividade ou a um escopo de interface morre com ela. Coroutines lançadas no escopo errado são fonte frequente de perda silenciosa: a tela fecha, o escopo cancela, o envio nunca termina.

Ao salvar estado de interface, lembre que há limite de tamanho para o que se guarda no pacote de estado; imagens e listas grandes não cabem. Guarde referências e recupere do armazenamento local. Teste o cenário de morte de processo com a opção de desenvolvedor que destrói atividades, e também deixando o app em segundo plano por bastante tempo.

Mudanças de configuração, como rotação, troca de idioma, modo escuro e tamanho de fonte, recriam a tela. Fluxos de captura em andamento precisam resistir a isso. E, com os modos de múltiplas janelas e telas dobráveis, o tamanho da janela pode mudar no meio de uma ação.

Por fim, observe notificações e serviços em primeiro plano: as exigências do sistema mudam a cada versão, e uma atualização do SDK alvo pode, de repente, bloquear algo que funcionava.

## Bateria, armazenamento e aparelhos modestos

O público é de motoristas, muitos com aparelhos de entrada, pouca memória e armazenamento quase cheio. Toda funcionalidade nova precisa ser pensada para esse perfil, não para o telefone de quem desenvolve. Carregar imagens grandes inteiras na memória, manter muitas bitmaps vivas ou fazer processamento pesado na thread principal causa travamentos e fechamentos inesperados que o motorista atribui ao app.

Armazenamento cheio é um caso real. O que o app faz quando não consegue gravar a foto? Se a resposta for "nada" ou "fecha", é um bug. Avise com clareza, preserve o que for possível e não finja que a captura deu certo. Do mesmo modo, limpe arquivos temporários e provas já confirmadas pelo servidor segundo uma política conhecida, mas jamais apague algo que ainda não foi confirmado.

Bateria: cada atualização de localização, cada nova tentativa de rede e cada animação contínua custa. Em um dia longo de rota, pequenas ineficiências se somam e o aparelho morre antes do último pacote. Medir consumo faz parte de validar mudanças em localização, sincronização e câmera.

Dados móveis também são limitados em muitos planos. Evite baixar recursos pesados sem necessidade, e considere que o envio de fotos é a maior parte do tráfego. Qualquer mudança que aumente o volume enviado por prova precisa ser pesada contra isso.

Por fim, teste com redes lentas e com latência alta de forma deliberada, usando ferramentas de limitação de rede, e não só com a rede boa do escritório.

## Migrações do banco local

O app guarda estado em banco local, e esse banco evolui. Cada mudança de estrutura precisa de migração escrita com cuidado, porque o aparelho do motorista pode ter dados que ainda não foram enviados. Uma migração destrutiva, ou o recurso de recriar o banco quando a versão não bate, apaga provas pendentes. Nunca use esse atalho fora de testes descartáveis.

Teste a migração partindo de bancos reais de versões anteriores, com fila preenchida e itens em estados diferentes (aguardando, em envio, com falha). Instalação limpa não prova nada sobre migração. Se houver saltos de várias versões, o caminho precisa funcionar também, pois alguns aparelhos ficam muito tempo sem atualizar.

Atenção à compatibilidade para trás: se o motorista precisar voltar a uma versão anterior (ou o sistema restaurar um backup antigo do app), um banco mais novo pode ser ilegível. Decida e documente o comportamento, em vez de deixar o app quebrar na abertura.

Outro ponto é o backup automático do sistema, que pode restaurar dados locais em outro aparelho. Dados de fila e de sessão geralmente não devem ser restaurados assim; confira o que está incluído e o que está excluído, e revise essa configuração quando adicionar novos tipos de dado local.

Evite operações longas de migração na thread principal; em bancos grandes, a abertura do app congela e o sistema pode oferecer encerrá-lo.

## Testes e como validar mudanças

Testes unitários passam facilmente sem cobrir o que realmente quebra aqui: conectividade instável, morte de processo, permissões revogadas, armazenamento cheio. Para mudanças em captura, fila, sincronização ou migração, planeje validação manual em aparelho físico, de preferência em mais de um fabricante e mais de uma versão do sistema, incluindo um aparelho modesto.

Use o emulador do Firebase para exercitar regras e consultas, mas lembre que ele não reproduz tudo do comportamento offline e da latência de produção. Mantenha casos de teste para rejeição de escrita por regra, para duplicidade de envio e para reenvio após falha parcial.

Testes instrumentados que dependem de câmera e de localização são frágeis; prefira isolar essas fontes atrás de interfaces e simular as respostas, deixando poucos testes ponta a ponta para o que for essencial. Mas não deixe que a abstração esconda os comportamentos reais do sistema, como atrasos e permissões negadas.

Quando corrigir um bug de campo, escreva um teste que o reproduza antes de consertar, sobretudo em perda de dados e duplicidade. Esses bugs costumam voltar com refatorações.

Por fim, desconfie de testes que dormem por tempo fixo para esperar sincronização; eles ficam instáveis e escondem condições de corrida. Prefira esperar por condições observáveis.

## Telemetria, privacidade e dados pessoais

O app lida com dados de terceiros: nome e endereço do destinatário, foto de fachada ou de pessoas, assinatura, posição. São dados pessoais, sujeitos a legislação de proteção de dados. Ao adicionar log, relatório de erro ou análise de uso, verifique o que vai junto. Foto, assinatura, endereço e coordenadas não devem aparecer em logs nem em eventos de telemetria.

A retenção também importa. Cópias locais de fotos e de assinaturas não devem ficar no aparelho além do necessário depois da confirmação do servidor. Revise isso ao mexer na limpeza de arquivos. E fotos salvas em áreas compartilhadas do armazenamento ou na galeria do usuário ficam expostas a outros aplicativos; mantenha-as em armazenamento privado do app.

Ao registrar erros, inclua contexto suficiente para diagnosticar (estágio do fluxo, tipo de falha, estado de conectividade) sem incluir conteúdo do usuário. Um bom registro de falha de envio ajuda muito mais que uma foto anexada a um relato.

Permissões pedidas devem ser as mínimas necessárias e explicadas no momento do uso. Adicionar uma permissão nova costuma exigir revisão de políticas das lojas de aplicativos e comunicação ao motorista; não faça isso de forma casual numa mudança pequena.

Por fim, qualquer envio de dados para serviços de terceiros, incluindo ferramentas de análise e de mapas, deve ser avaliado sob o mesmo prisma antes de entrar no pin-capture-app.

## Checklist rápido antes de abrir o PR

Perguntas curtas para fazer a si mesmo antes de enviar uma alteração no pin-capture-app. A mudança funciona do começo ao fim sem rede? A prova capturada é gravada de forma durável antes de qualquer outra etapa? O reenvio da mesma prova continua sendo inofensivo? Alguma consulta nova do Firestore precisa de índice ou de ajuste de regra, e isso já está no ambiente certo? O esquema Protocol Buffers continua compatível com versões antigas do app e do backend? Itens que já estão na fila local ainda são lidos depois da atualização? A migração do banco foi testada a partir de dados reais?

Continue: o que acontece com a mudança se o sistema matar o processo no meio? E se a permissão for revogada, o armazenamento estiver cheio ou o relógio do aparelho estiver errado? Há log ou evento novo que carregue dado pessoal? O consumo de bateria e de dados móveis piorou? A mudança foi vista em um aparelho modesto de verdade?

Se alguma resposta for "não sei", trate como "não". Esse componente fica na mão de quem trabalha na rua, e o custo de um erro é uma entrega sem prova, o que se traduz em disputa, retrabalho e perda de confiança. Prefira uma mudança menor e bem validada a uma grande e otimista, e deixe anotado aqui qualquer armadilha nova que você descobrir pelo caminho.
