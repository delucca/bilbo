---
id: 01M2G9GYAJE3QEKH5N8MYV945Z
created: 2026-09-14T12:45-03:00
---

# proof-upload-worker: resultado do teste de carga de upload de comprovantes

Esta nota substitui a nota anterior "proof upload load test" e traz o valor novo: depois de escalar o limite de instâncias, o proof-upload-worker aguentou `2000 uploads per minute` no pico, no lugar dos 1200 uploads per minute que constavam antes. Quem ler só esta nota já tem o número vigente. O valor antigo fica registrado aqui apenas para que ninguém o use por engano ao comparar com relatórios velhos.

O proof-upload-worker é o componente do backend do ParcelPin que recebe os comprovantes de entrega enviados pelo app Android dos entregadores, valida o conteúdo, grava os metadados no Cloud Firestore e guarda a mídia no armazenamento do Firebase. O app trabalha com suporte offline: o entregador captura a prova de entrega sem sinal, o app enfileira o envio e, quando a rede volta, uma enxurrada de uploads chega quase ao mesmo tempo. É esse padrão de rajada que o teste tentou reproduzir, e é por isso que o pico importa mais do que a média.

Escrevi esta nota com pressa, logo depois de fechar a rodada de testes. Ela é um relatório do que aconteceu e do que ainda precisa ser conferido, não um desenho de arquitetura. Onde eu não tinha certeza, digo que não tenho.

## Resultado principal

O número que importa é este: com o limite de instâncias escalado, o proof-upload-worker sustentou `2000 uploads per minute` no pico. O valor anterior, de 1200 uploads per minute, era o teto observado antes da mudança de configuração e não deve mais ser citado como capacidade atual do componente.

Vale deixar claro o que o número significa e o que ele não significa. Ele é o pico sustentado que o worker processou de ponta a ponta durante o teste, isto é, uploads recebidos, validados e gravados com sucesso. Não é um limite teórico nem um limite de projeto. Não encontramos, nesta rodada, o ponto em que o sistema quebra; encontramos o ponto até onde empurramos a carga com a configuração escalada e com o comportamento ainda dentro do aceitável. Se alguém quiser saber o teto real, precisa de outra rodada com carga maior.

A mudança que explica a diferença entre o valor antigo e o novo foi o aumento do limite de instâncias do serviço. Antes, o escalonamento automático chegava ao teto de instâncias cedo demais e as requisições excedentes ficavam esperando em fila ou eram recusadas e reenviadas pelo cliente. Com o limite mais alto, o serviço passou a abrir mais instâncias sob rajada e a fila deixou de crescer de forma descontrolada. Não mexemos na lógica de processamento para obter esse ganho; foi configuração de escala, e por isso o resultado é relativamente fácil de reproduzir e também fácil de perder se alguém reverter a configuração.

Como o ganho veio de escala horizontal, o custo também sobe junto no pico. Isso não foi medido com cuidado nesta rodada. Fica como pendência na seção de próximos passos.

### Comparação com o valor anterior

O valor antigo vinha da nota do teste de carga anterior. A diferença entre os dois valores é grande o bastante para não ser ruído de medição, mas não deve ser lida como melhoria de eficiência do código. A mesma instância, individualmente, processa aproximadamente o mesmo volume de antes; o que mudou foi a quantidade de instâncias disponíveis para absorver o pico.

Se aparecer um terceiro número em algum painel ou documento, desconfie primeiro de três coisas: a janela de medição usada, se o número conta uploads tentados ou uploads concluídos, e se a medição foi feita antes ou depois da mudança do limite de instâncias.

## Nome do componente

O nome atual é proof-upload-worker. O nome anterior era `uploadd`. Os dois se referem ao mesmo componente: o serviço de backend que processa os uploads de comprovantes de entrega. Não é um serviço novo nem um segundo serviço rodando em paralelo.

Isso importa na prática porque o nome antigo ainda aparece em lugares que ninguém atualizou. Ao procurar histórico, logs antigos, painéis de monitoramento, alertas, tickets e notas de testes passados, a busca por uploadd deve ser tratada como busca pelo proof-upload-worker. O contrário também vale: um documento novo que fale de proof-upload-worker pode estar descrevendo algo que no passado se chamava uploadd, e o histórico de incidentes dele está sob o nome velho.

A regra que adotei nas notas novas é usar sempre proof-upload-worker e mencionar uploadd só quando for necessário para ligar o passado ao presente, como aqui. Quem for renomear recursos que ainda carregam o nome antigo deve fazer isso com cuidado, porque alertas e filtros de log costumam depender do nome literal. Não revisei todos os lugares onde o nome antigo ainda existe; isso também está nos próximos passos.

## Como o teste foi montado

O objetivo era reproduzir o comportamento real dos entregadores no fim de um turno ou ao sair de uma área sem cobertura: muitos aparelhos recuperando a conexão em momentos próximos e esvaziando a fila local de comprovantes. Por isso a carga não foi uniforme. Ela subiu em rampa, ficou num patamar de pico por um tempo e depois desceu, para vermos tanto a absorção da rajada quanto a recuperação depois dela.

Os uploads de teste imitavam o formato real: um pedido com os metadados da entrega serializados com Protocol Buffers e uma imagem de comprovante anexa. Mantivemos o tamanho das imagens parecido com o que os aparelhos reais enviam, porque o tamanho da mídia pesa mais no tempo de cada requisição do que o restante do conteúdo. Os metadados seguem o mesmo esquema que o app Android usa em produção, então a validação do worker foi exercitada de verdade, e não só o caminho feliz.

O ambiente de teste usou um projeto Firebase separado do de produção, com o Cloud Firestore e o armazenamento próprios. Isso evitou poluir dados reais, mas tem uma limitação: o comportamento de cotas e de latência do Firestore pode diferir um pouco entre projetos, dependendo do histórico de uso. Trato o resultado como bom indicador, não como garantia exata para produção.

### Pontos que variamos

A principal variável foi o limite de instâncias do serviço. Rodamos com o limite antigo para confirmar que o comportamento antigo se reproduzia e depois com o limite escalado. Também observamos a concorrência por instância, mas não mudamos esse parâmetro entre as rodadas, justamente para que a diferença de resultado pudesse ser atribuída ao limite de instâncias e a mais nada.

Não variamos o tamanho das imagens nem a taxa de falhas simuladas de rede entre as rodadas. Ficou para uma próxima rodada, porque cada variável a mais dificulta a leitura do resultado.

### O que foi medido

Acompanhamos a taxa de uploads concluídos com sucesso, a latência de ponta a ponta de cada upload, o número de instâncias ativas ao longo do tempo, a taxa de erro devolvida ao cliente e o tamanho da fila de espera. Também olhamos o comportamento do lado do cliente, porque o app tem política de reenvio com espera crescente, e reenvios mal comportados podem transformar uma sobrecarga pequena numa grande.

## O que observamos

Com o limite antigo, o padrão era claro: a quantidade de instâncias chegava ao teto, a latência subia, a fila crescia e uma parte dos uploads voltava com erro temporário. O cliente reenviava, o que aumentava ainda mais a pressão por um tempo. O sistema se recuperava depois que a rajada passava, mas o tempo para esvaziar tudo era longo e a experiência do entregador, que fica olhando o app mostrar "pendente", era ruim.

Com o limite escalado, o serviço abriu instâncias suficientes para acompanhar a rampa. A latência subiu um pouco no começo do patamar de pico, enquanto novas instâncias iniciavam, e depois estabilizou. A fila não cresceu de forma sustentada. A taxa de erro ficou baixa e concentrada no início da rajada, quando ainda não havia instâncias prontas. Esse intervalo de partida a frio é a parte menos confortável do resultado, e discuto abaixo.

### Partida a frio

O atraso de iniciar instâncias novas é o principal custo de depender de escala automática. Durante a subida rápida, as primeiras requisições encontraram menos capacidade do que a necessária e pagaram o preço do início de instância. Como o app reenvia com espera crescente, o efeito para o usuário foi pequeno, mas não é zero. Manter um mínimo de instâncias aquecidas reduziria isso, ao custo de pagar por capacidade ociosa. Não decidimos nada sobre isso ainda.

### Firestore e armazenamento

O ponto que mais me preocupa para além do worker em si é o Cloud Firestore. Em cargas mais altas, a escrita dos metadados pode esbarrar em limites de taxa por documento ou por coleção, dependendo de como as chaves são distribuídas. Nesta rodada não vimos sinal disso, mas também não empurramos até o ponto onde apareceria. Se a próxima rodada for mais alta, esse é o primeiro lugar para olhar, antes de culpar o worker.

O armazenamento da mídia não apareceu como gargalo. Mesmo assim, o tempo de gravação da imagem domina a latência de cada requisição, então qualquer lentidão ali se reflete direto na capacidade efetiva por instância.

### Comportamento do cliente

O app Android, construído com Kotlin e Jetpack, usa uma fila local persistente para os comprovantes e um agendador de trabalho em segundo plano para enviar. Quando o servidor responde com erro temporário, o agendador tenta de novo com espera crescente. Isso se mostrou saudável sob o limite escalado e um pouco agressivo sob o limite antigo. Não alteramos nada no cliente; só registro que a política de reenvio é parte do sistema de carga e deve entrar em qualquer análise futura.

## Riscos e limites desta medição

Primeiro, o número de `2000 uploads per minute` vem de um ambiente de teste e de um perfil de carga sintético. Os entregadores reais têm padrões menos previsíveis, e a distribuição geográfica e de horário pode concentrar rajadas de formas que o teste não imitou. Tomo o valor como capacidade comprovada em teste, não como promessa para produção.

Segundo, o resultado depende da configuração de escala. Se alguém reduzir o limite de instâncias para economizar, a capacidade volta para perto do valor antigo. Vale proteger essa configuração com um comentário claro, um alerta ou uma revisão obrigatória, para que a mudança não seja desfeita sem querer.

Terceiro, não medimos custo. Mais instâncias no pico significam conta maior, e não sei dizer ainda se o custo do pico é aceitável para a frequência com que ele acontece. Essa conta precisa ser feita antes de tratar o valor novo como definitivo.

Quarto, a medição cobre o caminho normal de upload. Não cobre bem cenários de falha parcial, como o Firestore lento por um período, o armazenamento indisponível por alguns instantes ou uma rajada que chega no meio de uma implantação nova. Esses cenários merecem teste próprio, porque são justamente os que costumam gerar incidente.

Quinto, o nome antigo, uploadd, ainda pode estar presente em alertas, painéis e documentação. Se um alerta ainda filtra pelo nome antigo, ele pode não disparar para o proof-upload-worker. Isso é um risco operacional separado do teste de carga, mas apareceu enquanto eu juntava o material e por isso fica registrado aqui.

## Próximos passos

A lista abaixo está em ordem aproximada de importância. Nenhum item foi feito ainda.

Primeiro, repetir o teste com carga acima do valor atual para achar o teto real e ver qual parte quebra primeiro: o worker, o Firestore ou o armazenamento. Sem isso, o número registrado aqui é só um piso comprovado.

Segundo, estimar o custo do pico com o limite de instâncias escalado e comparar com a frequência real de rajadas em produção. Se o pico for raro, talvez compense aceitar um pouco de latência extra em vez de manter capacidade sobrando.

Terceiro, avaliar instâncias mínimas aquecidas para reduzir o efeito da partida a frio no começo das rajadas. Decidir com base no custo e no impacto percebido pelo entregador.

Quarto, procurar e atualizar todas as referências ao nome uploadd em alertas, painéis, filtros de log e documentação, trocando por proof-upload-worker, com atenção para não quebrar nenhum filtro que ainda dependa do nome literal. Fazer a troca de forma coordenada, de preferência confirmando que o alerta novo dispara antes de remover o antigo.

Quinto, testar cenários de falha parcial: Firestore lento, armazenamento indisponível e rajada durante implantação. Registrar o resultado numa nota própria, em vez de misturar com esta.

Sexto, variar o tamanho das imagens e a taxa de falhas de rede simuladas, para ver quanto a capacidade muda conforme o perfil de carga se afasta do que usamos.

### Como usar esta nota

Se a pergunta for "quanto o proof-upload-worker aguenta?", a resposta atual é `2000 uploads per minute` no pico, com o limite de instâncias escalado, e esse valor substitui o de 1200 uploads per minute da nota anterior sobre o teste de carga. Se a pergunta for "o que é uploadd?", é o nome antigo do mesmo componente, hoje chamado proof-upload-worker. Se a pergunta for sobre o teto real, o custo ou o comportamento em falha, esta nota não responde, e os próximos passos acima dizem como chegar lá.

Se alguém refizer o teste e obtiver outro número, a atitude certa é atualizar esta nota em vez de criar uma nova, anotando a configuração usada e a janela de medição, para que a comparação continue honesta.
