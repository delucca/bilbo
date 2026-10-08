---
id: 01M3BZ3NB98PY6101JH6ABKDG2
created: 2026-09-25T06:42-03:00
---

# Direção geral do results-ledger

O results-ledger vai ser o registro único e confiável de tudo que foi votado nas enquetes e nas perguntas do Q&A. Decidimos tratá-lo como um livro-razão: só se acrescenta, nunca se reescreve. Esta nota guarda a direção geral e o motivo, sem valores. Quem for mexer no componente deve ler antes de propor outro desenho.

## Contexto

O TownHall Pulse atende eventos virtuais grandes, com muita gente votando ao mesmo tempo. Produtores e gestores de comunidade precisam confiar no que aparece no painel. Se um resultado muda sem explicação no meio do evento, a confiança some e o suporte é acionado. O results-ledger existe para evitar isso.

Antes havia contagens espalhadas: uma no processo que atende os WebSockets, outra derivada no banco, outra montada no front em Next.js. Elas divergiam em momentos de pico. A decisão abaixo nasce desse problema.

## A decisão em uma frase

O results-ledger é a fonte de verdade dos resultados. Toda contagem exibida, em qualquer tela, é derivada dele e pode ser reconstruída a partir dele.

## Modelo de dados em linhas gerais

Cada voto, retirada de voto, aprovação ou rejeição de moderação vira um lançamento imutável. Contagens são projeções desses lançamentos. Correções entram como novos lançamentos que compensam os anteriores, nunca como edição do que já existe.

Isso dá trilha de auditoria de graça. Também deixa possível responder, depois do evento, o que cada pessoa viu em cada momento.

## Papel do CockroachDB

Os lançamentos ficam no CockroachDB, porque precisamos de consistência forte e de sobreviver à perda de um nó sem perder votos. A escolha de chaves deve evitar pontos quentes, já que muita gente vota na mesma enquete ao mesmo tempo. Detalhes de particionamento ficam para o código e para ajuste com medição real, não para esta nota.

## Papel do Elixir e do Phoenix

A escrita passa por processos Elixir que serializam lançamentos por enquete ou por sessão de Q&A. Assim a ordem é clara sem travar o sistema inteiro. O Phoenix só recebe o evento e entrega; ele não decide resultado.

## Tempo real e WebSockets

O painel ao vivo recebe atualizações empurradas pelos canais. A decisão é que o push seja uma visão agregada e pode agrupar atualizações para não inundar os clientes. A visão exata e final sempre vem do ledger. Se o canal perder mensagens, o cliente se resincroniza lendo o estado do ledger, sem precisar de replay completo.

## Moderação

A moderação em tempo real também escreve no ledger. Ocultar uma pergunta ou invalidar votos suspeitos é um lançamento, com autor e motivo. Nada é apagado de verdade. A tela decide o que mostrar com base no estado atual, mas o histórico fica inteiro para quem tem permissão de ver.

## Idempotência e repetição

Um mesmo voto pode chegar duas vezes por causa de reconexão ou retentativa. O ledger deve aceitar a repetição sem contar duas vezes. A forma de identificar a repetição é detalhe de implementação, mas a regra é fixa: reenviar é seguro.

## O que o front em Next.js pode fazer

O front só lê projeções e exibe. Ele não soma votos por conta própria e não guarda contagem como verdade. Pode fazer atualização otimista na interface, desde que troque pelo valor do ledger assim que ele chegar.

## Alternativas descartadas

Manter contadores mutáveis no banco foi descartado: são rápidos, mas perdem histórico e quebram sob concorrência. Manter a contagem só em memória nos processos também saiu, pela perda em caso de queda. Recalcular tudo a cada leitura foi rejeitado por custo em eventos grandes, então usamos projeções incrementais.

## Riscos conhecidos

- Projeções podem ficar atrasadas em relação ao ledger; precisamos tornar esse atraso visível e monitorado.
- O volume de lançamentos cresce rápido; será preciso uma política de retenção e arquivamento, a definir com produto.
- Reconstruir projeções em evento ao vivo pode competir com a carga normal; fazer isso com cuidado.
- Dados pessoais nos lançamentos exigem atenção de privacidade.

## Pontos em aberto

Ainda falta combinar com produto quanto tempo os lançamentos devem ficar acessíveis e em que formato os relatórios pós-evento saem. Também falta decidir como expor a trilha de auditoria para os gestores sem poluir a interface.

## Como usar esta nota

Se uma mudança proposta fizer qualquer tela mostrar um resultado que não saiu do results-ledger, ela contradiz esta decisão e precisa de nova discussão. Se a mudança só otimiza a leitura ou a entrega, provavelmente cabe dentro da direção atual. Em caso de dúvida, pergunte antes de criar um segundo lugar que guarde contagens.
