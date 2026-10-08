---
id: 01K2RS4K7BTETG4YK19T9P2YD8
created: 2025-08-16T03:30-03:00
---

# manifest-publisher: master.m3u8 publicado antes do último segmento causa HTTP 404

Se o manifest-publisher sobe o master.m3u8 antes de o último segmento chegar no S3, o player começa a tocar, pede a media playlist ou um segmento que ainda não existe e trava com `HTTP 404`. Já aconteceu em produção mais de uma vez e quase sempre foi diagnosticado como problema de CDN. Não é. A ordem de publicação é o problema. Esta nota registra o sintoma, a causa, como confirmar e o que fazer.

## Resumo rápido

O master.m3u8 só pode ficar público depois que tudo o que ele referencia já estiver no bucket: segmentos de todas as renditions, media playlists e o que mais for apontado por ele. Se o master sair antes, o player enxerga uma rendition que ainda está incompleta e recebe `HTTP 404`.

Nome: o componente se chama manifest-publisher. O nome antigo era `m3u8pub`. Se você achar `m3u8pub` em logs antigos, em dashboards, em alarmes, em nomes de state machine ou em scripts de deploy, é o mesmo componente que hoje se chama manifest-publisher. Ao buscar histórico, pesquise pelos dois nomes, senão metade dos incidentes não aparece.

## Sintoma

- O player abre o master, escolhe uma rendition e, ao pedir a media playlist ou os segmentos finais, recebe `HTTP 404`.
- O erro some sozinho depois de alguns segundos ou minutos, quando o último segmento termina de subir. Isso faz o bug parecer intermitente e difícil de reproduzir.
- Quem está tocando do começo muitas vezes não vê nada; o problema aparece em quem pula para o fim, ou em players que pré-carregam a playlist inteira.
- Em vídeos curtos o intervalo é pequeno e quase nunca aparece. Em vídeos longos ou com muitas renditions a janela é maior.
- Alguns players tentam de novo e se recuperam. Outros desistem na primeira falha e mostram erro ao usuário. Por isso o relato do cliente varia muito.

## Causa

O pipeline é orquestrado por AWS Step Functions. O transcode com FFmpeg gera os segmentos HLS de cada rendition em paralelo, e cada ramo sobe seus arquivos para o S3 no seu próprio ritmo. O manifest-publisher roda depois e escreve as playlists.

O defeito aparece quando o manifest-publisher é disparado por um sinal que não garante que todos os ramos terminaram. Casos conhecidos:

- Disparo pelo primeiro ramo concluído em vez do último.
- Disparo por evento de criação de objeto no S3 de uma playlist parcial.
- Retry do estado de publicação que reenvia o master enquanto um ramo ainda está subindo.
- Ordem de upload dentro do próprio publisher: o master saiu antes das media playlists por causa de upload concorrente sem barreira.

Consistência do S3 não é a explicação. Leitura depois de escrita é consistente. O 404 acontece porque o objeto realmente ainda não foi escrito.

## Como confirmar

1. Pegue o horário do erro nos logs do player ou do CDN.
2. Compare com o horário de criação do último segmento no bucket de saída. Se o master é mais antigo que o último segmento, é este bug.
3. Olhe a execução no Step Functions e veja se o estado do manifest-publisher começou antes de todos os ramos de transcode terminarem.
4. Procure nos logs pelos dois nomes, manifest-publisher e `m3u8pub`, porque execuções antigas registram o nome velho.

Se o master é mais novo que todos os segmentos e mesmo assim há 404, é outra coisa: caminho errado na playlist, cache negativo no CDN ou segmento apagado por lifecycle.

## Como evitar

- O estado de publicação só pode começar depois que todos os ramos de transcode e upload tiverem terminado com sucesso. Use a junção do estado paralelo, não um evento isolado.
- Dentro do publisher, suba primeiro segmentos e media playlists, espere todos os uploads confirmarem e só então suba o master. O master é sempre o último objeto escrito.
- Antes de subir o master, o publisher deve verificar que cada objeto referenciado existe no bucket. Se faltar algum, falhar de forma explícita em vez de publicar.
- Retries devem ser idempotentes e respeitar a mesma ordem. Um retry nunca pode reenviar o master sozinho antes de reconferir os segmentos.
- Não confiar em tempo de espera fixo. Já se tentou um sleep antes do master e ele só esconde o problema quando o vídeo é pequeno.

## Armadilhas ao corrigir

- Cache do CDN: se um master incompleto chegou a ser servido, o CDN pode guardar a resposta de erro por um tempo. Depois de corrigir, invalide o master e as playlists afetadas.
- Reprocessar o vídeo inteiro não é necessário na maioria dos casos. Basta rodar de novo apenas a publicação, na ordem certa, com os segmentos já no bucket.
- Cuidado com playlists de eventos ao vivo ou em andamento: aí o master pode legitimamente apontar para media playlists que crescem. Este gotcha vale para o fluxo de vídeo sob demanda, onde o conjunto é fechado.
- Não troque a ordem só no código do publisher e esqueça a definição da state machine. As duas camadas precisam garantir a barreira.

## Relacionado

Um problema que se confunde com este é o de arquivos de entrada truncados, que gera falhas de análise antes do transcode e também termina em playlists estranhas. Veja [[probe-analyzer-fails-truncated]]. A diferença: lá o erro nasce na análise da mídia de origem; aqui a mídia está boa e o que falha é a ordem de publicação.

## Pendências

- Adicionar um teste de integração que atrasa de propósito um ramo de upload e confere que o master só aparece depois.
- Alarme quando houver `HTTP 404` em media playlist logo após uma publicação recente.
- Renomear o que ainda se chama `m3u8pub` em dashboards, alarmes e scripts, para que a busca por manifest-publisher traga tudo.
