---
id: 01KHXSAK4C2KJTV257HZKCNAZZ
created: 2026-02-20T12:05-03:00
---

# pulse-socket: reconexão em loop quando o idle_timeout do balanceador é menor que o heartbeat

Se o `idle_timeout` do load balancer for menor que o intervalo de heartbeat do pulse-socket, os clientes veem `close code 1006` e reconectam em loop. Isso parece um bug do servidor ou do cliente Next.js, mas a causa é só a configuração do balanceador na frente do pulse-socket. Perdemos tempo olhando o lado errado; esta nota existe para não repetir isso.

## Sintoma

Em eventos grandes, os participantes veem a enquete ou o Q&A congelar, o indicador de conexão piscar e a tela voltar sozinha. No console do navegador aparece `close code 1006` repetidas vezes. Esse código significa fechamento anormal: o cliente não recebeu um frame de close de verdade. A conexão simplesmente caiu por baixo, sem aviso do servidor.

O padrão que denuncia o problema:

- A conexão abre normalmente e a reconexão funciona.
- A queda acontece depois de um tempo parecido a cada tentativa, sem relação com a ação do usuário.
- Nada de estranho no log do Phoenix: o processo do canal não registra crash nem erro.
- O ciclo repete até alguém mexer na configuração.

## Causa

O pulse-socket mantém a conexão WebSocket viva com heartbeat. Se o heartbeat é mais espaçado que o `idle_timeout` do balanceador, a conexão fica ociosa por mais tempo do que o balanceador tolera. O balanceador então corta o TCP por conta própria. Nem o servidor Elixir nem o navegador mandam um close frame, e por isso o cliente enxerga `close code 1006`.

O cliente reconecta, o novo socket volta a ficar ocioso além do limite, o balanceador corta de novo. Daí o loop. Quanto mais clientes, mais reconexões ao mesmo tempo, o que também pesa no servidor e no banco (CockroachDB) por causa de re-subscribe e reidratação do estado do evento.

## Como confirmar

1. Compare o intervalo de heartbeat configurado no pulse-socket com o `idle_timeout` do balanceador. O heartbeat precisa ser claramente menor.
2. Veja se as quedas ocorrem sempre após o mesmo tempo de ociosidade. Se o tempo é constante e bate com o `idle_timeout`, está confirmado.
3. Teste direto no pulse-socket, sem passar pelo balanceador. Se o loop some, o balanceador é o culpado.
4. Olhe os logs de acesso do balanceador: eles costumam mostrar a conexão encerrada por timeout de ociosidade.

Um trecho de referência de como pensar na relação entre os valores (nomes ilustrativos, não são chaves reais do projeto):

```text
heartbeat  <  idle_timeout   -> ok
heartbeat  >= idle_timeout   -> close code 1006 + reconexão em loop
```

## Correção

Há dois caminhos, e o ideal é garantir a relação nos dois lados:

- Aumentar o `idle_timeout` do balanceador para um valor folgado acima do heartbeat.
- Diminuir o intervalo de heartbeat do pulse-socket para ficar com margem abaixo do `idle_timeout`.

Deixe margem. Se os dois valores forem quase iguais, jitter de rede e atraso de agenda do BEAM fazem o heartbeat chegar tarde de vez em quando, e o loop volta de forma intermitente, o que é pior de diagnosticar.

## Armadilhas

- Existe mais de um balanceador ou proxy no caminho em alguns ambientes (CDN, ingress, balanceador de nuvem). Vale o menor `idle_timeout` de toda a cadeia, não só o do primeiro salto.
- Mudar o `idle_timeout` em um ambiente e esquecer outro: staging funciona, produção entra em loop em dia de evento grande.
- Um backoff agressivo no cliente esconde o problema em testes pequenos, mas não resolve nada em escala.
- Não confundir com outros motivos de `close code 1006`, como queda real de rede do usuário ou deploy reiniciando nós. A diferença é que aqui o padrão é regular e afeta todos os clientes parecidos.

## Prevenção

Registrar a relação entre heartbeat e `idle_timeout` como requisito de infraestrutura. Qualquer mudança em um dos dois valores deve revisar o outro. Antes de eventos grandes, rodar um teste com conexões ociosas por mais tempo que o heartbeat e conferir que nenhuma cai. Se a equipe de produção de eventos relatar enquete congelando com reconexões, esta é a primeira coisa a checar no pulse-socket.
