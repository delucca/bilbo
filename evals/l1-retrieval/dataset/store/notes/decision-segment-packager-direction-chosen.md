---
id: 01JTPF9JNE3ZFBQ798NGZM8617
created: 2025-05-07T19:54-03:00
---

# Direção geral do segment-packager

Nota rápida sobre o rumo que o time escolheu para o `segment-packager`. Nada aqui fixa valores; é só a direção. Quem for mexer no componente deve ler isto antes de propor mudança grande.

## Contexto

O `segment-packager` recebe as variantes já transcodificadas pelo FFmpeg e monta o que o player precisa para tocar em HLS. O público são equipes de operação de mídia em editoras independentes, sem time de infraestrutura grande. Então simplicidade de operação pesa mais que ganho marginal de desempenho.

## Decisão principal

O `segment-packager` fica como etapa separada, pequena e sem estado próprio, chamada pelo fluxo orquestrado no Step Functions. Ele não decide a escada de qualidade nem faz transcodificação. Só empacota o que recebe e grava o resultado.

## Por que separado

Separar deixa repetir só o empacotamento quando algo falha, sem refazer a transcodificação, que é a parte cara. Também deixa o componente fácil de testar sozinho com entradas pequenas.

## Linguagem e implementação

O código segue em Rust. O FFmpeg continua sendo chamado como processo externo quando for preciso remuxar, e não ligado como biblioteca. Isso isola falhas do FFmpeg e evita acoplar a build ao ciclo de lançamentos dele.

## Entrada e saída

A entrada vem do S3, produzida pela etapa anterior. A saída volta para o S3, em uma estrutura previsível por vídeo e por variante. O `segment-packager` não depende de nada local que sobreviva entre execuções.

## Idempotência

Rodar duas vezes sobre a mesma entrada deve dar o mesmo resultado e sobrescrever sem deixar lixo. Isso é o que torna as tentativas automáticas do orquestrador seguras. Qualquer mudança que quebre isso precisa de discussão antes.

## Playlists

As playlists são geradas por nós, a partir de metadados confiáveis das variantes, e não extraídas por inspeção solta dos arquivos. A playlist mestra só é publicada depois que todas as variantes estão completas, para o player nunca ver uma escada pela metade.

## Segmentos

A duração dos segmentos segue a mesma política em todas as variantes, para o alinhamento entre elas não quebrar a troca de qualidade. A política em si fica na configuração do pipeline, não no código do `segment-packager`.

## Falhas

Falha de entrada ruim é erro definitivo e sobe com causa clara para quem opera. Falha transitória de rede ou do S3 é repetida pelo orquestrador. O componente não faz repetição escondida por conta própria.

## Observabilidade

Cada execução registra em log estruturado o vídeo, a variante e a etapa em que parou. Operador de mídia precisa achar o motivo sem ler código. Mensagens devem falar a língua do domínio, não de FFmpeg cru.

## Segurança de dados

O componente usa só as permissões de leitura e escrita nos locais de que precisa. Nada de acesso amplo ao armazenamento. Conteúdo de editoras é tratado como sensível.

## Compatibilidade

Priorizamos a saída que players comuns toleram bem, mesmo que isso renuncie a recursos novos do HLS. Recurso extra só entra se não prejudicar os players mais antigos que os clientes ainda usam.

## O que ficou de fora

- Empacotamento em outros formatos de streaming.
- Proteção de conteúdo dentro do próprio `segment-packager`.
- Reencode para corrigir entrada ruim.

## Alternativas descartadas

Juntar empacotamento e transcodificação na mesma etapa foi descartado: acopla falhas e encarece cada nova tentativa. Usar o FFmpeg como biblioteca também ficou de lado pelo acoplamento de build.

## Riscos conhecidos

Chamar processo externo exige cuidado com saída, códigos de retorno e processos pendurados. Fica como ponto de atenção em revisão de código.

## Quando rever

Rever esta decisão se aparecer necessidade real de outro formato de entrega, ou se o custo de chamar o FFmpeg como processo virar problema medido. Sem medição, não mudar por gosto.

## Próximos passos

Manter esta nota em dia quando algo mudar na direção. Valores concretos ficam na configuração e nos testes, não aqui.
