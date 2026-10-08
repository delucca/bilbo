---
id: 01JS1XGQD1THQB3YYF8SHK621E
created: 2025-04-17T10:02-03:00
sources:
  - "code: crates/segment-packager/src/hls.rs"
---

# Design do segment-packager

O segment-packager é a etapa do ReelForge que pega as renditions já transcodificadas da escada de bitrate adaptativo e as empacota em HLS para streaming. Esta nota registra como ele corta os segmentos e por que o desenho é assim. Escrevi com pressa, então é mais um registro do estado atual do que um documento polido.

## Resumo da decisão

O segment-packager corta segmentos HLS de 10 segundos, usando o argumento `-hls_time 10` do FFmpeg. Esse é o único valor de duração de segmento usado por ele. Todas as renditions da escada passam pelo mesmo valor, para que os limites de segmento fiquem alinhados entre elas.

## Contexto

O ReelForge recebe vídeo enviado por equipes de operações de mídia de publishers independentes. O vídeo é transcodificado em várias renditions e depois empacotado. O segment-packager fica no fim desse caminho: ele não decide bitrate nem resolução, só fatia e gera as playlists. A orquestração é feita por AWS Step Functions, e a entrada e a saída ficam no AWS S3.

## Por que 10 segundos

A duração do segmento é um compromisso entre três coisas:

- Segmentos longos reduzem o número de requisições e o tamanho das playlists.
- Segmentos curtos deixam a troca de rendition mais rápida quando a banda do espectador muda.
- Cada segmento precisa começar em um keyframe, então a duração real depende de como o encode foi feito.

Para o público típico dos publishers independentes, com conteúdo sob demanda e não ao vivo, o valor de `-hls_time 10` dá um bom equilíbrio. Não há exigência de latência baixa aqui, então não compensa pagar o custo de muitos objetos pequenos no S3.

## Alinhamento com keyframes

O FFmpeg só corta em keyframe. Se o intervalo de keyframes do encode não for compatível com a duração pedida, os segmentos saem com tamanhos diferentes do esperado, e isso quebra o alinhamento entre renditions. Por isso a etapa de transcodificação precisa gerar keyframes em intervalos que dividam bem a duração do segmento. O segment-packager assume isso e não reencoda para corrigir.

## Exemplo de uso

Trecho ilustrativo da parte do comando que define o corte. Os demais argumentos dependem da rendition e ficam de fora.

```sh
ffmpeg -i entrada.mp4 -hls_time 10 -hls_playlist_type vod saida.m3u8
```

## Interação com Step Functions e S3

O segment-packager roda como um passo do fluxo no Step Functions. Ele lê as renditions de um prefixo no S3 e grava segmentos e playlists em outro. Como a duração é fixa, o fluxo não precisa passar esse parâmetro adiante; se um dia for configurável, o valor deve entrar na entrada do passo e não ficar escondido no código Rust.

## Riscos e pontos de atenção

- Mudar o valor de `-hls_time 10` afeta todo conteúdo novo, mas não o que já foi publicado. Misturar durações em uma mesma escada causa problemas de troca entre renditions.
- Conteúdo muito curto pode terminar com um último segmento menor que o normal. Isso é esperado.
- Se o keyframe não bater com o corte, o player pode se comportar de forma estranha na troca de qualidade. Vale conferir primeiro o encode antes de culpar o empacotamento.

## Pendências

- Avaliar se algum publisher precisa de segmentos mais curtos, por exemplo para conteúdo ao vivo.
- Documentar como validar o alinhamento de segmentos entre renditions depois do empacotamento.
- Decidir se a duração passa a ser parâmetro do fluxo ou continua fixa no segment-packager.
