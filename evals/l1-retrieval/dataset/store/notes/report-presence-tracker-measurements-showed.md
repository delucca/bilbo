---
id: 01M0QMC2DWBAGA0BFJ8F3EYSGH
created: 2026-08-23T12:38-03:00
sources:
  - "doc: presence capacity test"
---

# presence-tracker: medição de memória por nó

Anotação rápida sobre o consumo de memória do `presence-tracker` em carga alta. A medição mostrou `310 MB` de memória por nó com `15000 attendees` por nó. Esse é o número de referência até alguém medir de novo com outra configuração.

## Resumo do achado

O `presence-tracker` usou `310 MB` por nó quando cada nó segurava `15000 attendees`. Ou seja, o custo por participante fica bem baixo, na casa de poucas dezenas de KB. Não é o gargalo que a gente temia antes do teste. Quem for dimensionar nós para um evento grande pode partir desse valor, com folga.

## Contexto

O `presence-tracker` é o componente do TownHall Pulse que sabe quem está conectado a um evento: enquanto a pessoa está na sala, ela conta como presente. Roda em Elixir sobre Phoenix, com as conexões chegando por WebSockets. Produtores de evento e gestores de comunidade usam essa contagem no painel, e a moderação em tempo real também depende dela.

A dúvida de origem era simples: quanta memória cada nó precisa para eventos grandes, já que a presença é mantida em processos vivos e replicada entre nós do cluster.

## Como foi medido

A medição foi feita por nó, olhando o uso de memória da VM do Erlang com o nó já estabilizado, depois que os participantes entraram. Não foi um teste longo de soak. O valor reflete o estado estável com os participantes conectados, não o pico durante a entrada em massa.

Detalhes que ficam em aberto: não anotei o perfil exato de entrada e saída durante o teste, nem se havia muita atividade de enquetes e perguntas ao mesmo tempo. Tratar o número como ordem de grandeza confiável, não como garantia.

## O que o número cobre

- Memória do `presence-tracker` por nó, com `15000 attendees` por nó.
- Estado de presença mantido localmente e o que vem da replicação entre nós.
- Não cobre o resto da aplicação Phoenix no mesmo nó, como canais de enquete e Q&A.
- Não cobre o Next.js, que roda separado.

## Limites e dúvidas

Só há um ponto de medição. Não sabemos ainda se o crescimento é linear acima de `15000 attendees` por nó. Em sistemas de presença com replicação, o custo pode crescer mais rápido quando o cluster aumenta, porque cada nó guarda parte do estado dos outros. Isso precisa ser testado, não presumido.

Também não está claro quanto a memória sobe durante a reconexão em massa, por exemplo quando um nó cai e os clientes voltam todos juntos. Esse cenário pode passar do valor estável.

## Impacto no dimensionamento

Com `310 MB` por nó nessa carga, a memória do `presence-tracker` não deve decidir o tamanho das máquinas. Provavelmente CPU, número de conexões abertas e fan-out de mensagens aparecem antes. Para planejar eventos grandes, vale olhar esses fatores primeiro e usar este valor só como parcela fixa na conta.

## Próximos passos

- Repetir a medição com mais participantes por nó, para ver se a curva continua linear.
- Medir o pico durante entrada em massa e durante reconexão após queda de nó.
- Medir com atividade real de enquetes e Q&A rodando junto.
- Registrar o resultado aqui, corrigindo esta nota se algo divergir.

## Referências

Esta nota é a única fonte da medição por enquanto. Se surgir um relatório de teste de carga mais completo, ligar aqui. Componente: `presence-tracker`. Banco do projeto: CockroachDB, que não entra neste número, porque a presença fica em memória nos nós.
