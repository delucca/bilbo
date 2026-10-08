---
id: 01KH371PAG3S40QETARDSCDTZ2
created: 2026-02-10T04:25-03:00
---

# signature-pad-view: referência de consumo e uso

Nota de referência sobre o `signature-pad-view`, o componente de captura de assinatura do app de entregadores do ParcelPin. A coordenada Gradle para consumir o `signature-pad-view` é `com.parcelpin:signature-pad:2.3.0`. Quem for adicionar o componente a outro módulo do app Android deve declarar essa coordenada exata na dependência do módulo. A nota foi escrita com pressa, a partir do que se sabe do projeto, e mantém os detalhes gerais de propósito. Onde faltar precisão, confira no código antes de afirmar algo para outra pessoa.

## O que é e para que serve

O `signature-pad-view` é uma view Android, escrita em Kotlin, que mostra uma área em branco onde o destinatário de uma entrega desenha a assinatura com o dedo ou com uma caneta capacitiva. É uma das peças da prova de entrega do ParcelPin, ao lado da foto do pacote, da localização e do horário. O app é usado por motoristas de última milha, muitas vezes em rua, com luz forte, mão suada, tela rachada e sinal ruim. O componente precisa funcionar nessas condições sem exigir nada especial do motorista.

A coordenada de consumo é `com.parcelpin:signature-pad:2.3.0`. O nome do artefato publicado (`signature-pad`) é mais curto que o nome do componente neste projeto (`signature-pad-view`). Não são coisas diferentes: o artefato é como o componente chega ao app via Gradle. Se alguém procurar por `signature-pad-view` no repositório de dependências e não achar, deve procurar pelo artefato com a coordenada acima.

O componente faz só a parte visual e de captura. Ele não decide onde a assinatura é guardada, não envia nada para a rede e não conhece o Firestore. Essa separação é intencional: a view devolve o traço capturado para quem a hospeda, e o fluxo de prova de entrega cuida da persistência, da fila de envio e da sincronização. Isso mantém o componente fácil de testar e permite reaproveitá-lo em telas que não têm relação com entrega.

### Como a view é usada na tela de prova de entrega

Na tela de prova de entrega, a view fica dentro de um layout com um botão para limpar e um botão para confirmar. O motorista pede a assinatura, o destinatário desenha, e o motorista confirma. Antes da confirmação, a tela consulta a view para saber se há traço suficiente. Uma assinatura vazia ou com um simples toque não deve ser aceita como prova. O critério de traço suficiente é interno ao componente e pode mudar entre versões, então a tela não deve duplicar essa regra.

Depois da confirmação, a tela pede à view uma representação da assinatura. Há mais de uma forma de obter o resultado: uma imagem rasterizada e uma lista dos pontos do traço. O fluxo de prova de entrega usa os pontos do traço como dado principal, porque eles são pequenos e se prestam bem a serialização com Protocol Buffers. A imagem serve para exibir a assinatura em telas de conferência e para anexar ao comprovante.

## Integração com o restante do app

O app usa Jetpack, e o componente se encaixa nesse modelo de duas formas. Primeiro, a view guarda e restaura o próprio estado quando a Activity ou o Fragment é recriado, por exemplo numa rotação de tela ou depois que o sistema mata o processo em segundo plano. Perder a assinatura no meio da captura é uma das queixas mais irritantes dos motoristas, porque obriga o destinatário a assinar de novo. Segundo, o estado da captura pode ser lido por um ViewModel, que o mantém durante mudanças de configuração. A recomendação é que o ViewModel da tela de prova de entrega seja o dono do resultado, e a view seja só a superfície de desenho.

### Dependência no Gradle

A declaração é feita no módulo que contém a tela de prova de entrega, usando a coordenada `com.parcelpin:signature-pad:2.3.0`. Se houver catálogo de versões no projeto, a versão deve ficar no catálogo, e o módulo deve referenciar o alias. O importante é que todos os módulos que usam o componente apontem para a mesma versão. Misturar versões diferentes do artefato no mesmo app costuma gerar classes duplicadas ou comportamento de desenho diferente entre telas, e esse tipo de erro só aparece em aparelhos específicos.

Ao atualizar a versão, o caminho seguro é este: ler as notas de lançamento do artefato, trocar a versão em um único lugar, compilar, rodar os testes de interface da tela de prova de entrega e abrir a captura num aparelho real. Emulador não basta, porque a sensação do traço (suavização, latência, pressão) depende do hardware de toque.

### Relação com o armazenamento offline

O ParcelPin foi desenhado para funcionar sem rede. O motorista conclui a entrega, a prova fica gravada localmente, e o envio acontece quando houver conexão. O Cloud Firestore, com a persistência offline do Firebase, faz parte desse caminho, e os dados estruturados da prova passam por mensagens definidas em Protocol Buffers. A assinatura entra nesse fluxo como um campo da prova de entrega. Como a view não fala com o Firestore, nada no componente depende de haver conexão. Isso é uma propriedade que vale proteger: se algum dia o componente passar a exigir rede para funcionar, o modo offline do app quebra num ponto crítico, que é a hora de entregar o pacote.

Um cuidado prático é o tamanho do que se grava. A lista de pontos do traço cresce com o tempo de desenho e com a taxa de amostragem. Se a amostragem for muito alta, a mensagem fica grande à toa. Vale conferir, ao atualizar o componente, se a amostragem padrão mudou, e se o fluxo de prova de entrega ainda simplifica o traço antes de serializar. Imagens rasterizadas grandes devem ir para armazenamento de arquivos, não dentro do documento, para não estourar limites de tamanho de documento e para não deixar a sincronização lenta em rede móvel fraca.

## Comportamentos conhecidos e cuidados

Esta seção junta o que normalmente dá problema com uma view de assinatura em app de entrega. São observações gerais; confirme cada uma no código e nos testes antes de tratá-la como garantia da versão atual.

### Gestos e rolagem

A view fica dentro de telas que rolam. Quando o destinatário começa a desenhar, o gesto não pode ser roubado pelo contêiner de rolagem, senão o traço sai cortado ou a tela sobe e desce junto. O componente deve pedir ao pai que não intercepte o toque enquanto o dedo está sobre a área de desenho. Se aparecer um bug de traço interrompido, o primeiro lugar a olhar é a hierarquia de views ao redor, não o componente em si. Layouts novos, com contêineres de rolagem aninhados, já causaram esse tipo de comportamento em outros projetos.

### Tela, densidade e orientação

Os aparelhos dos motoristas variam muito em tamanho e densidade. O traço deve ter espessura coerente em qualquer tela, e a área de desenho deve ter proporção parecida em retrato e paisagem. Se a proporção da área mudar entre captura e exibição, a assinatura aparece esticada no comprovante. A recomendação é guardar, junto com o traço, as dimensões da área em que ele foi desenhado, para que a exibição posterior possa reescalar sem distorcer.

### Acessibilidade

Uma assinatura desenhada é difícil para quem tem limitação motora ou visual. O fluxo de prova de entrega precisa ter uma alternativa aceita pela operação, como nome legível do recebedor ou foto, para esses casos. O componente em si deve expor uma descrição de conteúdo adequada e permitir limpar a área por um controle acessível. Isso é decisão de produto e de operação, não só de código, então não mude a regra sozinho.

### Privacidade

A assinatura é dado pessoal. Não deve aparecer em logs, em relatórios de falha nem em ferramentas de análise. Ao depurar, evite registrar a lista de pontos ou a imagem. Se for necessário reproduzir um problema, use traços sintéticos de teste. Também vale lembrar que o cache local do aparelho guarda a prova até o envio; a limpeza desse cache depois da sincronização é responsabilidade do fluxo de prova de entrega, não do componente.

### Desempenho

O desenho acontece na thread principal, em resposta a eventos de toque. Aparelhos mais fracos mostram atraso no traço quando a view redesenha a área inteira a cada ponto novo, ou quando a tela tem muita coisa pesada ao redor. Se houver reclamação de traço atrasado, medir antes de mudar: abrir o perfil de renderização do aparelho, ver se o gargalo é o desenho da view ou o restante da tela, e só então decidir. Uma atualização do componente pode melhorar ou piorar isso, então o teste em aparelho de entrada, do tipo que os motoristas realmente usam, faz parte da validação de cada nova versão.

## Como verificar e o que fazer em caso de dúvida

Para confirmar qual versão do componente o app está usando, olhe a declaração de dependência do módulo da tela de prova de entrega, ou o catálogo de versões, e procure a coordenada `com.parcelpin:signature-pad:2.3.0`. Se o valor encontrado for diferente, esta nota está desatualizada ou o módulo está fora do padrão; em qualquer dos casos, corrija a nota ou o módulo, conforme o que a equipe decidir.

Checklist rápido ao mexer em algo que toque o componente:

- Conferir que todos os módulos usam a mesma versão do artefato.
- Testar a captura em aparelho real, em retrato e paisagem.
- Girar a tela no meio do traço e confirmar que o desenho sobrevive.
- Confirmar que a captura funciona com o aparelho em modo avião.
- Verificar que uma assinatura vazia continua sendo recusada.
- Verificar que nada da assinatura vai para logs ou análise.
- Conferir o tamanho do que é gravado na prova de entrega após a mudança.

### Perguntas que esta nota deve ajudar a responder

Como o `signature-pad-view` chega ao app? Pela dependência Gradle `com.parcelpin:signature-pad:2.3.0`. O componente precisa de rede? Não; ele só captura e devolve o traço, e quem persiste e sincroniza é o fluxo de prova de entrega, apoiado no Firestore com suporte offline. Em que formato o resultado é usado? Principalmente como lista de pontos serializada com Protocol Buffers, com a imagem como apoio para exibição. Quem é dono do estado da captura? O ViewModel da tela hospedeira, com a view apenas desenhando e restaurando o próprio estado.

### Pendências desta nota

Falta registrar, quando alguém tiver o código aberto, o nome exato da classe da view, os atributos de layout aceitos, o critério interno de traço suficiente e o histórico de mudanças entre versões do artefato. Esses itens foram deixados de fora de propósito para não afirmar nada que não tenha sido conferido. Quem preencher deve citar o arquivo de origem e a versão do componente a que a informação se refere, para que a próxima atualização do artefato possa ser comparada com o que está escrito aqui.

Se a equipe decidir trocar de biblioteca de assinatura ou internalizar o componente, esta nota deve ser atualizada no mesmo dia, mantendo a coordenada antiga como registro histórico e indicando a nova. Notas de referência que ficam desatualizadas em silêncio são piores do que nenhuma nota, porque quem as lê confia nelas.
