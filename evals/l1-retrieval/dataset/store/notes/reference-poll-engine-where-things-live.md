---
id: 01KGY7CEQQC2DZG2X2EJV8D4HK
created: 2026-02-08T05:55-03:00
---

# poll-engine: onde ficam as peças

Nota rápida de mapa para o `poll-engine` do TownHall Pulse. Não é especificação nem decisão: só diz onde procurar cada coisa quando for mexer. Se algo mudou de lugar, corrija aqui.

## Visão geral

O `poll-engine` cuida do ciclo de vida das enquetes em eventos virtuais grandes: criar, abrir, receber votos, fechar e publicar o resultado. A parte de servidor é Elixir com Phoenix. O painel de quem produz o evento e a tela do público são em Next.js. A conversa entre os dois lados é por WebSockets.

## Lado Elixir

A lógica de negócio das enquetes fica no contexto Phoenix do `poll-engine`, separada da camada web. Procure primeiro nos módulos de contexto (regras de abrir, fechar, contar) e só depois nos controllers. Processos que seguram estado de uma enquete em andamento ficam sob a supervisão da aplicação, normalmente um por enquete ativa. Se o comportamento parece "perdido" depois de um restart, comece por aí.

## Tempo real

Os canais Phoenix do `poll-engine` ficam na camada web, junto dos outros canais do produto. O socket cuida de autenticar a conexão e de entrar no tópico da enquete. Os eventos que o cliente recebe (voto contabilizado, enquete fechada, resultado parcial) são emitidos a partir do contexto, não dos canais. Ao mudar o formato de uma mensagem, ajuste também o cliente Next.js.

## Persistência

Os dados vão para o CockroachDB, via Ecto. Esquemas e migrações ficam no diretório de migrações do app Elixir. Votos e enquetes têm tabelas próprias. Lembre que transações podem ser repetidas pelo banco em caso de conflito, então código de escrita de voto precisa tolerar retry.

## Moderação

A moderação em tempo real toca o `poll-engine` em poucos pontos: ocultar uma enquete, encerrar à força e bloquear votos suspeitos. Essas ações passam pelo mesmo contexto das ações normais, para que os eventos de canal saiam iguais. Não existe um caminho paralelo para moderadores; se achar um, provavelmente é legado.

## Frontend Next.js

Componentes de enquete ficam separados em duas áreas: a do produtor (criar, controlar, ver resultado) e a do público (votar). O hook ou módulo que abre o WebSocket é compartilhado e fica numa pasta de utilitários do app. Para depurar, olhe primeiro o que chega no socket antes de culpar a tela.

## Exemplo de busca

Para localizar rápido os pontos de contato, algo assim costuma bastar:

```bash
grep -ri "poll-engine" .
```

## Pendências de mapeamento

- Confirmar onde ficam os testes de carga, se existirem.
- Anotar quem é dono do esquema de votos.
- Registrar a configuração de ambientes quando for estável.
