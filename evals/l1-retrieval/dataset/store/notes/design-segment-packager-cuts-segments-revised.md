---
id: 01KHBSBYGNYZ4NMGXWWAQQGHJS
created: 2026-02-13T12:19-03:00
---

# Design do segment-packager: duração dos segmentos HLS

Esta nota substitui a nota anterior, "segment packager cuts segments". O valor novo é `-hls_time 6`: o segment-packager corta segmentos HLS de 6 segundos, e não mais de 10 segundos como antes. A mudança existe para acelerar o início da reprodução.

O componente se chamava `slicepack` antes. Hoje o nome é `segment-packager`. Se você achar `slicepack` em log antigo, dashboard, runbook, mensagem de commit ou conversa de equipe, é o mesmo componente. Nos textos novos, use só `segment-packager`.

## Contexto

O ReelForge recebe vídeo enviado pelo usuário, transcodifica para uma escada de bitrates adaptativos e empacota o resultado para streaming. Quem usa são equipes de operações de mídia em editoras independentes. Elas não costumam ter engenheiro de vídeo dedicado. Por isso o padrão precisa ser bom sem ajuste manual, e a configuração precisa ser fácil de explicar.

O `segment-packager` fica no fim da cadeia. Ele pega as renditions já codificadas, corta em segmentos, gera as playlists HLS e entrega tudo no S3. A orquestração fica com o AWS Step Functions. O corte em si é feito pelo FFmpeg, chamado por código em Rust.

Esta nota registra a decisão sobre a duração do segmento, o motivo, o que muda e o que não muda, e o que conferir quando alguém mexer nisso de novo.

## Decisão

A duração alvo do segmento passa a ser de 6 segundos, pedida ao FFmpeg com `-hls_time 6`. O valor anterior era de 10 segundos.

A decisão vale para todas as renditions da escada. Não há valor diferente por rendition. Se um dia houver, isso pede uma nota nova, porque quebra a premissa de que os limites de segmento coincidem entre renditions.

```sh
ffmpeg -i <entrada> -hls_time 6 <playlist>.m3u8
```

O trecho acima só mostra onde entra a opção. A linha de comando real do `segment-packager` tem outras opções, montadas no código Rust, e não está copiada aqui.

## Motivo: início mais rápido

O motivo é o tempo até o primeiro quadro. Um player HLS precisa baixar a playlist e pelo menos o primeiro segmento antes de tocar. Com segmento menor, esse primeiro download é menor e o início fica mais rápido.

A ideia vem de reclamações das equipes de operações sobre o vídeo demorar a começar, principalmente em conexões fracas e em celulares. O segmento mais curto ataca isso de forma direta e barata, sem mexer na codificação.

Não medimos o ganho em produção antes de decidir. A decisão se apoia no comportamento conhecido do HLS. Veja a seção sobre como validar.

## O que mudou em relação à nota antiga

A nota "segment packager cuts segments" dizia que o corte era de 10 segundos. Isso não vale mais. Ela também usava o nome `slicepack` em alguns trechos, que agora é `segment-packager`.

O resto do desenho de empacotamento continua como estava: o formato de saída é HLS, a entrega vai para o S3 e a etapa é chamada pelo Step Functions. Só a duração do segmento e o nome mudaram.

Se você achar a nota antiga em alguma busca, trate a duração dela como superada. Esta nota é a referência.

## Nome do componente

O componente se chamava `slicepack` e hoje é `segment-packager`. A troca foi só de nome. O comportamento do componente não mudou por causa dela.

Pontos onde o nome antigo pode sobrar:

- logs e métricas antigos, que ficam com o nome da época;
- documentação e runbooks que ninguém atualizou;
- nomes de máquinas de estado, de alarmes e de painéis, se foram criados antes da troca;
- comentários no código e mensagens de commit.

Ao procurar histórico, busque pelos dois nomes. Ao escrever algo novo, use `segment-packager`. Não crie alias novo para `slicepack`.

## Como o corte funciona

O FFmpeg corta o HLS em pontos que respeitam quadros-chave. A opção `-hls_time 6` é um alvo, não uma garantia exata. O segmento real termina no primeiro quadro-chave depois que o alvo foi atingido. Se os quadros-chave forem mais espaçados que o alvo, os segmentos ficam mais longos que 6 segundos.

Por isso a duração do segmento e o intervalo de quadros-chave da transcodificação andam juntos. Com alvo de 6 segundos, o ideal é que o intervalo de quadros-chave divida o alvo de forma limpa. Assim os segmentos saem uniformes e iguais em todas as renditions.

Essa dependência é o ponto mais fácil de esquecer. Veja a seção sobre alinhamento com o transcodificador.

## Alinhamento com o transcodificador

A etapa de transcodificação define os quadros-chave. O `segment-packager` apenas corta. Se a transcodificação deixar quadros-chave em lugares que não combinam com `-hls_time 6`, o resultado são segmentos de tamanhos irregulares ou playlists com durações declaradas que variam muito.

Antes de aceitar esta mudança como pronta, confira em um vídeo de teste que os limites de segmento caem nos mesmos instantes em todas as renditions da escada. Isso é o que permite ao player trocar de rendition sem soluço.

Se a transcodificação mudar o intervalo de quadros-chave, reavalie o alvo do segmento. As duas coisas formam uma decisão só.

## Efeitos no número de segmentos

Segmento mais curto significa mais segmentos por vídeo. Para um mesmo vídeo, a quantidade de objetos no S3 sobe de forma proporcional à razão entre o valor antigo e o novo. Isso afeta:

- o número de requisições de escrita na hora do empacotamento;
- o número de requisições de leitura na hora da reprodução;
- o tamanho das playlists, que ganham mais linhas;
- o custo de operações no S3, que cresce com a contagem de requisições, não com os bytes.

Esse aumento é o preço aceito pelo início mais rápido. Em vídeos longos ele aparece mais. Vale acompanhar o custo de requisições nas primeiras semanas.

## Efeitos na eficiência de codificação

O HLS exige que cada segmento comece em um quadro-chave. Segmentos mais curtos obrigam a ter mais quadros-chave por minuto, e quadro-chave custa mais bits que quadro predito. Na prática, para a mesma qualidade, o arquivo total tende a ficar um pouco maior.

Esse efeito foi considerado e aceito. Para o público do ReelForge, que são publicações independentes com vídeos de duração variada, o início rápido pesa mais que uma pequena perda de eficiência.

Se alguém quiser recuperar eficiência depois, o caminho é mexer na transcodificação, não voltar ao valor antigo sem discutir.

## Efeitos no player

Players HLS costumam manter um buffer medido em segmentos ou em tempo. Com segmentos menores, o buffer em número de segmentos cresce. Em geral isso é inofensivo, mas há players com configuração fixa em número de segmentos. Nesses casos o buffer em tempo diminui e a reprodução fica mais sensível a oscilação de rede.

A troca de rendition também reage mais rápido, porque há mais pontos de decisão. Isso costuma ser bom para a adaptação de bitrate. O contraponto é que decisões mais frequentes podem causar mais trocas de qualidade visíveis, dependendo do algoritmo do player.

Não controlamos o player das editoras. Se uma equipe reclamar de troca de qualidade excessiva depois da mudança, essa é a primeira hipótese a checar.

## Efeitos no cache e na CDN

Objetos menores e em maior quantidade mudam o perfil de cache. A taxa de acerto por requisição tende a se manter, mas o número total de entradas no cache aumenta. Para conteúdo muito popular isso não importa. Para cauda longa de vídeos pouco vistos, pode haver mais falhas de cache no começo, porque cada segmento é um objeto separado.

O primeiro segmento é o mais pedido de cada vídeo. Como agora ele é menor, o custo de uma falha de cache no começo da reprodução também é menor. Esse é um dos motivos pelos quais o início fica mais rápido.

## Efeitos no Step Functions

A máquina de estados que chama o `segment-packager` não depende da duração do segmento. Ela passa a entrada e espera a conclusão. O que pode mudar é o tempo de execução da etapa, já que há mais segmentos a gravar no S3.

Se a etapa de empacotamento tiver limite de tempo configurado, confira se ele ainda sobra para vídeos longos. Não houve mudança conhecida de limite por causa desta decisão, mas vale conferir ao revisar a máquina de estados.

Também vale observar o tamanho da saída da etapa. Se alguma versão da máquina de estados guardar a lista de segmentos no estado, ela cresce com a mudança e pode esbarrar nos limites de tamanho do serviço. O desenho recomendado é guardar só o local da playlist, não a lista de segmentos.

## Efeitos no S3

Os segmentos e as playlists são gravados no S3. Com mais objetos por vídeo, a gravação em paralelo ajuda a manter o tempo da etapa. O `segment-packager` deve continuar tratando falha de gravação de objeto individual com nova tentativa, sem refazer o vídeo inteiro.

As regras de ciclo de vida e de limpeza do bucket operam por prefixo e por idade, então não dependem da duração. Mesmo assim, apagar um vídeo agora remove mais objetos, e a operação de limpeza leva mais tempo.

## Compatibilidade com vídeos já publicados

Vídeos já empacotados com o valor antigo não são reprocessados automaticamente. Eles continuam com segmentos de 10 segundos e continuam funcionando. A mudança vale para empacotamentos novos.

Isso significa que, por um tempo, o acervo mistura duas durações. Nenhum player HLS conforme a especificação tem problema com isso, porque cada playlist declara a duração dos próprios segmentos.

Reempacotar o acervo antigo é uma decisão separada. Só faz sentido se as equipes pedirem início mais rápido nos vídeos antigos. Não está planejado.

## Como validar

A validação pensada para esta mudança:

- empacotar um vídeo de teste e conferir que as durações declaradas na playlist ficam próximas de 6 segundos;
- conferir que os limites de segmento coincidem entre as renditions;
- medir o tempo até o primeiro quadro em um player real, comparando com um vídeo empacotado do jeito antigo;
- olhar a contagem de objetos no S3 e o custo de requisições nas primeiras semanas;
- ouvir as equipes de operações sobre a percepção de início e de troca de qualidade.

Ainda falta a medição comparativa de início em produção. Quando alguém fizer, vale atualizar esta nota com o resultado, sem inventar números: registre o que foi medido e como.

## Alternativas consideradas

Manter o valor antigo era o caminho sem risco, mas deixava o início lento, que é a queixa principal.

Usar um valor ainda menor ajudaria mais no início, mas aumentaria demais a contagem de objetos, o custo de requisições e a perda de eficiência de codificação. O valor escolhido é um meio-termo.

Usar um primeiro segmento curto e os demais longos é uma técnica conhecida. Não adotamos porque complica o alinhamento entre renditions e o código do `segment-packager`, e o ganho adicional não justificava a complexidade agora. Pode voltar à mesa se o início continuar sendo problema.

## Riscos e pontos em aberto

- Falta medir o ganho real de início em produção.
- O custo de requisições no S3 pode subir mais do que o esperado em vídeos longos.
- Players com buffer fixo em número de segmentos podem ter menos folga em tempo.
- O intervalo de quadros-chave da transcodificação precisa continuar compatível com o alvo; ninguém verifica isso de forma automática hoje.
- Documentação antiga ainda cita `slicepack` e o valor de 10 segundos.

## Quando revisar esta decisão

Revise se acontecer qualquer uma destas coisas: a medição de produção não mostrar ganho de início; o custo do S3 subir de forma incômoda; as equipes relatarem troca de qualidade excessiva; ou a transcodificação mudar a política de quadros-chave.

Ao revisar, mantenha o nome `segment-packager`, registre o valor anterior (`-hls_time 6`) e o motivo da mudança, e atualize esta nota em vez de criar outra sobre o mesmo assunto.
