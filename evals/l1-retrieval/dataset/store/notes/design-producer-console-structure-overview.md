---
id: 01K674Y756QGQS4NMQD1SMRCJA
created: 2025-09-28T00:12-03:00
---

# Estrutura do producer-console

O producer-console é a parte do TownHall Pulse que o produtor de evento e o community manager usam durante a transmissão. É um app Next.js que conversa com o backend Phoenix por WebSocket. Esta nota descreve só a estrutura geral, sem valores fixos. Escrita com pressa, então confira no código antes de confiar em algum detalhe.

## Visão geral

O console fica entre o produtor e o evento ao vivo. Ele mostra enquetes, perguntas e o estado da moderação em tempo real. Quase tudo que aparece na tela vem de eventos empurrados pelo servidor, não de consultas repetidas.

## Camada de interface

Next.js com componentes de página por área: enquetes, perguntas, fila de moderação e painel do evento. A navegação é simples, uma página por área, com um layout comum que guarda a conexão e o contexto do evento atual.

## Conexão em tempo real

Uma conexão WebSocket por sessão do produtor, aberta no layout comum. Ela entra nos canais Phoenix do evento e distribui as mensagens para as telas por um store local. Se a conexão cai, o console tenta reconectar e pede o estado atual de novo.

```text
producer-console (Next.js) <-> WebSockets <-> Phoenix (Elixir) <-> CockroachDB
```

## Backend Phoenix

Os canais ficam do lado Elixir. Cada evento tem seu tópico, e o console só recebe o que o seu papel permite. Comandos do produtor (abrir enquete, aprovar pergunta) chegam como mensagens no canal e são validados no servidor.

## Persistência

O estado durável vive no CockroachDB. O console não acessa o banco direto, sempre passa pelo Phoenix. Leituras iniciais vêm por requisição HTTP, atualizações depois vêm pelo canal.

## Enquetes

Tela para criar, abrir, fechar e acompanhar resultados. Os resultados parciais chegam agregados pelo servidor; o console só desenha. Não recalcula votos no cliente.

## Perguntas e Q&A

Lista das perguntas enviadas pelo público, com ordenação e destaque da que está no ar. O produtor escolhe qual vai para a tela do evento.

## Fila de moderação

Perguntas e mensagens pendentes entram numa fila. O moderador aprova, rejeita ou marca. A ação é enviada ao servidor e a interface só muda quando a confirmação volta, para evitar divergência entre moderadores.

## Estado no cliente

Um store central guarda o evento atual, as listas e o estado da conexão. As telas assinam fatias dele. Atualizações otimistas são evitadas nas ações de moderação.

## Papéis e permissões

Produtor e community manager têm visões parecidas, com permissões diferentes. A checagem real é no servidor; o console só esconde o que o papel não pode usar.

## Autenticação

A sessão é validada antes de abrir o canal. Se expirar durante o evento, o console avisa e pede nova entrada sem perder o contexto da tela.

## Tratamento de erros

Erros de comando aparecem como aviso na própria tela. Erros de conexão aparecem num indicador global. Evitar telas bloqueadas, porque o produtor precisa seguir operando ao vivo.

## Desempenho

Eventos grandes geram muitas mensagens. O console agrupa atualizações antes de renderizar e usa listas virtualizadas onde o volume é alto. Vale medir antes de mexer nisso.

## Testes

Testes de componentes para as telas e testes de integração contra um backend de desenvolvimento para os fluxos de canal. Falta cobertura melhor de reconexão.

## Pontos de atenção

- Ordem das mensagens no canal pode diferir da ordem na tela se o store não tratar atrasos.
- Reconexão precisa ressincronizar, não só reabrir o canal.
- Qualquer regra nova de moderação deve ser aplicada no servidor primeiro.

## Pendências

Documentar o contrato dos eventos do canal em um lugar só e revisar a divisão entre o store e as páginas.
