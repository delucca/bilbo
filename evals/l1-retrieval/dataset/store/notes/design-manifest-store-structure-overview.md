---
id: 01KWGHCTBCTF92C9EPR1E8BEZQ
created: 2026-07-02T01:29-03:00
---

# manifest-store: estrutura geral do componente

Nota rápida sobre como o manifest-store é organizado hoje. É o componente que guarda no aparelho do entregador o manifesto de entregas do dia (paradas, volumes, janelas, instruções) e o mantém em sincronia com o backend. O app inteiro depende dele para saber o que mostrar na tela de rota e o que ainda falta entregar. Escrevi isto para quem for mexer no código sem ter participado do começo. Não tem valores nem decisões fechadas aqui, só o desenho geral e as armadilhas que a gente já conhece.

A ideia central: o app nunca lê o manifesto direto do Firestore na interface. A interface lê do armazenamento local, e o manifest-store cuida de trazer e levar dados em segundo plano. Isso é o que permite o motorista trabalhar em porão de prédio, estrada ruim ou sem plano de dados.

## Responsabilidades

O manifest-store tem poucas responsabilidades, e vale mantê-las assim:

- Receber o manifesto do backend e transformá-lo em registros locais consultáveis.
- Expor para o restante do app uma leitura reativa das paradas e dos volumes, na ordem da rota.
- Aceitar mudanças de estado feitas pelo entregador (chegou, entregue, falhou, reagendado) e registrá-las de forma durável antes de qualquer envio.
- Enviar essas mudanças ao backend quando houver conexão, com repetição segura.
- Reconciliar o que veio do servidor com o que foi alterado localmente.
- Limpar o que não serve mais, respeitando o que ainda tem pendência de envio.

Tudo que não cabe nessa lista deveria morar em outro lugar. Quando alguém propõe colocar regra de negócio de rota ou de cobrança aqui dentro, a resposta padrão é não.

## O que fica fora do escopo

O componente não captura foto, assinatura, código de barras nem localização. Isso é do módulo de captura de prova de entrega. O manifest-store apenas guarda a referência de que existe uma prova associada a uma parada e o estado de envio dela.

Também não otimiza rota. A ordem das paradas vem pronta do backend, e o componente só a preserva. Se o motorista reordena manualmente, isso entra como uma alteração local como qualquer outra e passa pela mesma fila.

Autenticação também não é com ele. O componente recebe uma identidade já resolvida e a usa para escolher o escopo de dados. Se a sessão cai, ele para de sincronizar e mantém o que já tem, sem apagar nada por conta própria.

## Modelo de dados do manifesto

O manifesto é composto por alguns níveis. No topo há o manifesto em si, que representa o conjunto de trabalho de um entregador para um período de operação. Dentro dele há paradas, e cada parada pode ter vários volumes. A parada carrega endereço, contato, janela de atendimento, observações do destinatário e o estado atual. O volume carrega identificação para conferência e o estado individual.

A separação entre parada e volume importa porque a entrega pode ser parcial: parte dos volumes entregue, parte devolvida. O estado da parada é derivado do estado dos volumes mais eventos explícitos do motorista, e não o contrário. Quem quebrou essa regra antes acabou com paradas marcadas como concluídas e volumes ainda pendentes.

Cada entidade tem um identificador estável vindo do backend e, quando criada localmente, um identificador provisório que depois é associado ao definitivo. O código não deve assumir que os dois são iguais.

## Camadas internas

O componente segue a divisão usual de apps Android modernos, com ajustes nossos:

- Camada de fontes de dados: uma fonte local e uma fonte remota, cada uma escondendo sua tecnologia.
- Camada de repositório: é a única API pública. Combina as duas fontes, aplica as regras de reconciliação e expõe fluxos observáveis.
- Camada de trabalho em segundo plano: agendamentos de sincronização e de envio de pendências.
- Camada de mapeamento: converte entre os formatos remoto, local e de domínio.

O restante do app só enxerga interfaces do repositório e modelos de domínio. Nenhum tipo do banco local ou do Firestore deve vazar para ViewModel ou tela. Em revisão de código, vazamento desse tipo é motivo para pedir mudança.

## Armazenamento local

O armazenamento local é um banco relacional embutido, acessado pelas bibliotecas do Jetpack. Ele é a fonte da verdade para a interface. As tabelas seguem o desenho do modelo: manifestos, paradas, volumes, mais tabelas de apoio para pendências de envio e para metadados de sincronização.

A interface assina consultas reativas, então qualquer escrita local, vinda do servidor ou do motorista, reflete na tela sem chamada extra. As consultas já devolvem as paradas na ordem da rota e com os agregados necessários (quantos volumes faltam, por exemplo), para evitar cálculo na tela.

Migrações de esquema são tratadas com cuidado especial, porque um aparelho de motorista pode ficar muito tempo sem atualizar o app e ter dados pendentes dentro dele. Migração destrutiva não é aceitável quando existem pendências não enviadas. Se for inevitável recriar algo, as pendências precisam ser preservadas ou exportadas antes.

## Sincronização com o Cloud Firestore

O lado remoto usa o Cloud Firestore. Os documentos do manifesto ficam organizados por entregador e por período de operação, com as paradas e volumes como subcoleções ou campos aninhados, conforme o caso. A escolha do formato de cada parte considerou o custo de leitura e o tamanho dos documentos, sem que o desenho fosse otimizado a fundo.

O componente evita manter ouvintes abertos o tempo todo. O padrão é puxar mudanças em momentos definidos (abertura do app, retorno de conectividade, ação do usuário, execução periódica em segundo plano) e aplicar o resultado em transação local. Ouvintes em tempo real só entram quando há uma razão clara, como o despachante reatribuindo paradas com o motorista na rua.

O cache offline nativo do Firestore existe, mas não é a base do funcionamento. Ele pode ficar ativo como camada auxiliar, mas o app não depende dele para mostrar dados. Foi uma escolha para ter controle sobre o que é guardado, por quanto tempo e como conflitos são tratados.

## Protocol Buffers e contratos

Parte do conteúdo trafega e é persistida em mensagens definidas em Protocol Buffers: o corpo do manifesto entregue pelo backend, os eventos de mudança de estado e alguns blobs guardados no banco local. A vantagem é um contrato único entre app e backend, com geração de código em Kotlin e compatibilidade para frente e para trás se as regras de evolução forem seguidas.

Regras que a gente tenta respeitar: não reaproveitar identificadores de campo removidos, nunca mudar o tipo de um campo existente, tratar todo campo novo como opcional no leitor, e ignorar campos desconhecidos em vez de falhar. Como há aparelhos com versões antigas do app em campo, o leitor precisa tolerar mensagens mais novas do que ele conhece.

Os modelos gerados ficam restritos à camada de mapeamento. O domínio tem suas próprias classes Kotlin, para que mudanças no contrato não se espalhem pelo app.

## Fluxo de download do manifesto

Quando o entregador abre o dia, o repositório pede ao backend o manifesto atual. A resposta é validada (estrutura mínima, consistência entre paradas e volumes) e aplicada ao banco local numa única transação. Se algo falha no meio, nada é aplicado e a versão anterior continua visível.

O download é incremental sempre que possível: o componente guarda um marcador do último estado conhecido e pede apenas o que mudou desde então. Quando o marcador não é aceito pelo servidor, ou quando a consistência local é duvidosa, ele cai para um download completo e reconcilia.

Durante o download, a interface não fica bloqueada. Se já existe manifesto local, ele continua sendo mostrado, com um indicador discreto de atualização em andamento. Só no primeiro uso, sem nada local, a tela espera.

## Fluxo de atualização de estado

Quando o motorista marca uma parada, o repositório escreve duas coisas na mesma transação local: o novo estado da entidade e um registro de pendência descrevendo a mudança. A tela atualiza na hora, a partir do estado local, sem esperar rede.

O registro de pendência é um evento, não um retrato do estado inteiro. Isso facilita a repetição e a reconciliação, porque o servidor recebe o que aconteceu e quando, e não só o resultado. Cada evento carrega o instante em que ocorreu no aparelho e uma chave que permite ao servidor reconhecer repetições.

Depois da escrita, o repositório pede ao agendador que processe a fila. Se não houver conexão, o pedido fica registrado e o trabalho roda quando as condições permitirem.

## Fila de pendências e offline

A fila de pendências é a parte mais sensível. Ela precisa sobreviver a fechamento do app, reinício do aparelho e falta de bateria. Por isso vive no banco local, não em memória, e só perde um item depois da confirmação do servidor.

O processamento respeita a ordem dos eventos por parada, porque enviar uma conclusão antes de uma chegada pode gerar estado incoerente no backend. Entre paradas diferentes, a ordem é mais livre. Falhas transitórias (rede, indisponibilidade) levam a nova tentativa com espera crescente. Falhas definitivas (rejeição por regra de negócio) tiram o item da fila normal e o marcam para tratamento visível, sem repetição infinita.

O envio de provas de entrega (mídia pesada) é separado do envio do evento de estado. O evento pode subir primeiro, com a prova marcada como em envio, para que o backend tenha a informação essencial mesmo quando a mídia demora. O manifest-store acompanha apenas o estado dessa associação.

## Conflitos e reconciliação

Conflito acontece quando o servidor muda algo que o motorista também mexeu offline: reatribuição de parada, cancelamento, mudança de janela, correção de endereço. A regra geral é que o que o motorista fez na rua é um fato, e o que o servidor decidiu é uma instrução. Os dois precisam ser preservados, não sobrescritos às cegas.

Na prática, o componente aplica as mudanças do servidor sobre os campos que o motorista não alterou e mantém os eventos pendentes na fila. Se o servidor remove uma parada que já tem evento local, o caso é sinalizado para tratamento em vez de sumir silenciosamente. Entregas realizadas nunca devem desaparecer da visão do motorista por causa de uma atualização remota.

As regras detalhadas de reconciliação ficam concentradas no repositório, num lugar só, com testes próprios. Espalhar essa lógica por fontes de dados ou telas foi uma fonte de bugs no passado.

## Integração com a captura de prova

O módulo de captura consome o manifest-store por uma interface pequena: obter a parada atual, registrar que uma prova foi criada, consultar o estado de envio. Ele não escreve direto nas tabelas.

A prova só conta como parte da entrega depois de associada à parada e ao evento correspondente. Se a captura é interrompida no meio, o componente não deve ficar com referência órfã. Uma rotina de limpeza confere periodicamente referências sem arquivo e arquivos sem referência, e trata cada caso de forma conservadora, sem apagar o que ainda possa ser necessário para envio.

## Ciclo de vida e uso do Jetpack

O acesso reativo é feito com fluxos do Kotlin expostos pelo repositório e coletados nas ViewModels, respeitando o ciclo de vida da tela. O trabalho em segundo plano usa o agendador do Jetpack para tarefas que precisam sobreviver ao processo, com restrições de rede apropriadas. Tarefas curtas disparadas pela interface rodam em escopo próprio do repositório, não no escopo da tela, para que sair da tela não cancele um envio em curso.

Injeção de dependências entrega ao repositório as fontes de dados e os agendadores. Em testes, as fontes são trocadas por versões em memória. Nada no componente deve depender de singleton global escondido.

O componente também precisa lidar com o sistema matando o processo a qualquer momento. Toda operação multi-etapa é desenhada para ser retomável: ou termina por inteiro na transação, ou deixa um registro suficiente para continuar depois.

## Segurança e regras de acesso

O acesso remoto é restrito por regras do Firebase, de forma que cada entregador lê e escreve apenas o que lhe pertence. O app não tenta ser a barreira; ele assume que o servidor valida. Do lado local, o banco guarda dados pessoais de destinatários, então vale tratá-lo como sensível: não registrar conteúdo em logs, não exportar em relatórios de falha e apagar dados quando a sessão é encerrada de forma definitiva.

Quando o entregador troca de conta no mesmo aparelho, o componente separa os dados por identidade, e a troca nunca pode expor o manifesto de uma pessoa à outra. Pendências não enviadas de uma conta anterior merecem atenção especial: ou são enviadas com a identidade correta, ou ficam retidas e sinalizadas, nunca descartadas em silêncio.

## Observabilidade

Para investigar problemas de campo, o componente emite eventos de diagnóstico sobre o próprio funcionamento: início e fim de sincronização, tamanho aproximado da fila, falhas por categoria, uso do caminho de reconciliação. Esses eventos não carregam dados de destinatários.

O sinal mais útil até agora é a idade da pendência mais antiga. Quando ela cresce, há algo errado com envio, com permissões do sistema ou com restrições de bateria do fabricante do aparelho. Esse último caso é comum em alguns modelos de Android, que encerram trabalho em segundo plano de forma agressiva, e já gerou muito chamado de suporte.

## Testes

A cobertura se divide em três grupos. Testes unitários do repositório e das regras de reconciliação, com fontes falsas, cobrem os cenários de conflito. Testes instrumentados do banco local verificam consultas, transações e migrações, incluindo migração com pendências presentes. Testes de integração com o emulador do Firebase exercitam o caminho remoto sem tocar em dados reais.

Cenários que sempre devem ter teste: processo morto no meio de uma escrita, conexão que cai durante o envio, mesma mudança enviada duas vezes, servidor removendo parada com evento local, mensagem com campos desconhecidos.

## Pontos de atenção e dívidas conhecidas

- A fronteira entre estado da parada e estado dos volumes ainda é mal explicada em alguns pontos do código; vale consolidar a regra de derivação em um lugar só.
- A tela de pendências com falha definitiva é simples demais para o suporte diagnosticar casos sem ajuda de engenharia.
- A limpeza de dados antigos poderia ser mais previsível; hoje depende de gatilhos espalhados.
- Parte do mapeamento entre contrato e domínio é manual e repetitiva, e é onde campos novos costumam ser esquecidos.
- Falta documentar melhor, para quem integra, quais fluxos do repositório são seguros para chamar da interface e quais só fazem sentido em segundo plano.

Se for alterar algo estrutural no manifest-store, leia primeiro as seções de fila e de conflitos. É onde um erro pequeno vira entrega perdida em campo.
