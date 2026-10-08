---
id: 01JV4PRTRAV08D1XV8SYV6RYXX
created: 2025-05-13T08:34-03:00
sources:
  - "doc: Outbox Runbook"
---

# sync-outbox-relay: referência de operação e drenagem

Anotação de referência sobre o `sync-outbox-relay`, o componente do LabNotebook Sync que lê as linhas pendentes do outbox no SQL Server e as entrega ao RabbitMQ, para que o resto do sistema veja as entradas do caderno eletrônico e a saída dos instrumentos. Serve para quem vai operar, dar manutenção ou depurar o relay. Não é um design completo: só o que costuma ser preciso na hora. Quando algo aqui divergir do código, o código vence e esta nota deve ser corrigida.

O comando que mais importa: para esvaziar as linhas pendentes antes de uma manutenção, rode `dotnet run --project src/OutboxRelay -- --drain` para o `sync-outbox-relay`. O resto da nota explica quando usar, o que ele faz e o que olhar depois.

## O que o sync-outbox-relay faz

O sistema grava a mudança de negócio e a mensagem a ser publicada na mesma transação do SQL Server. A mensagem fica numa tabela de outbox, como linha pendente. O `sync-outbox-relay` é o processo separado que varre essa tabela, publica cada linha no RabbitMQ e marca a linha como entregue. Assim a gravação no banco e a publicação não podem divergir de forma silenciosa: ou a linha está no outbox esperando, ou já foi publicada e marcada.

A garantia é de entrega pelo menos uma vez. Se o relay publicar e cair antes de marcar a linha, ela será publicada de novo na próxima varredura. Por isso os consumidores precisam ser idempotentes. Isso vale para qualquer consumidor novo que alguém ligar ao barramento: não presuma que cada mensagem chega uma única vez.

O relay carrega mensagens de dois tipos de origem, em termos gerais:

- alterações em entradas do caderno eletrônico feitas por cientistas;
- arquivos e leituras produzidos por instrumentos, cujo conteúdo pesado fica no Azure Blob Storage e cuja mensagem leva só a referência.

O relay não decide regra de negócio nem de auditoria. Ele só transporta. A trilha de auditoria depende de que nenhuma linha se perca e de que a ordem por entrada seja respeitada, e é nisso que o relay precisa ser confiável.

## Ordem e garantias

A ordem importa dentro de uma mesma entrada de caderno: uma correção não pode chegar antes da criação que ela corrige. O relay lê as linhas pendentes em ordem de gravação e publica nessa ordem. Entre entradas diferentes não há garantia de ordem, e ninguém deve depender disso.

Cuidados que já pegaram gente:

- Se houver mais de uma instância do relay rodando ao mesmo tempo contra o mesmo banco, a ordem por entrada pode quebrar, a menos que a leitura use travamento adequado das linhas. Antes de subir uma segunda instância, confira como a reserva das linhas está feita no código e não confie na memória.
- Uma linha que falha repetidamente não deve travar a fila inteira sem aviso. Verifique como o relay trata a linha venenosa na versão atual: se ele pula, se para, ou se move para um estado de erro. Isso mudou durante o desenvolvimento e vale reler o código antes de afirmar qualquer coisa a um auditor.
- A marcação de entregue acontece depois da confirmação do broker. Sem confirmação do RabbitMQ a linha continua pendente.

Para compliance, o ponto central: pendente no outbox significa que a mudança já está gravada e auditada no banco, mas ainda não chegou a quem consome. Isso é atraso, não perda.

## Drenagem antes de manutenção

Antes de qualquer manutenção que pare o relay, o broker ou o banco, esvazie o que está pendente. O comando é este:

```bash
dotnet run --project src/OutboxRelay -- --drain
```

O modo de drenagem processa as linhas pendentes do `sync-outbox-relay` e termina quando não restam mais, em vez de ficar rodando como serviço contínuo. É o que se quer antes de uma janela de manutenção: sair dela sem um acúmulo de mensagens presas e sem depender de que o serviço normal reinicie direito.

Sequência que costumo seguir:

1. Avisar quem usa o sistema, em especial a equipe de compliance, que haverá janela. Entradas novas podem continuar sendo gravadas, só a entrega aos consumidores atrasa.
2. Parar a origem de novas gravações, se a manutenção exigir banco quieto. Se não exigir, aceitar que sempre pode sobrar uma ou outra linha nova.
3. Rodar o comando de drenagem acima e esperar o término. Ler a saída até o fim.
4. Conferir no banco que não há mais linhas pendentes no outbox, com uma consulta simples de contagem do que ainda não foi marcado como entregue.
5. Só então parar o serviço normal do relay e fazer a manutenção.
6. Depois da manutenção, subir o serviço normal e acompanhar a primeira varredura.

Se a drenagem terminar mas ainda sobrarem linhas pendentes, não siga em frente como se nada tivesse acontecido. Veja a seção de problemas abaixo.

## Quando a drenagem não esvazia

Causas mais comuns, da mais provável para a menos provável:

- **Broker indisponível ou recusando publicação.** O RabbitMQ pode estar com alarme de recurso ativo, ou a conexão pode estar caindo. O relay não marca como entregue sem confirmação, então as linhas ficam. Resolva o broker primeiro e rode a drenagem de novo.
- **Credenciais ou permissões.** Depois de rotação de segredos, o relay pode estar tentando com credencial antiga, para o banco ou para o broker. A mensagem de erro costuma deixar claro qual dos dois.
- **Linha que não serializa ou é rejeitada.** Uma mensagem malformada, ou grande demais para o que o broker aceita, falha toda vez. Olhe a linha específica no outbox.
- **Referência a blob ausente.** Para mensagens de instrumento, a mensagem aponta para um objeto no Azure Blob Storage. Se o relay valida a existência antes de publicar e o blob ainda não subiu, a linha espera. Confirme se o upload do instrumento terminou.
- **Travas no SQL Server.** Uma transação longa de outra parte do sistema pode segurar linhas do outbox e impedir o relay de ler ou marcar. Procure bloqueios antes de culpar o relay.

Depois de corrigir a causa, rode a drenagem outra vez. Ela é segura de repetir: por causa da semântica de pelo menos uma vez, repetir pode no máximo duplicar uma publicação, que o consumidor deve tolerar.

Não apague linhas do outbox à mão para fazer a contagem zerar. Isso destrói a garantia de auditoria. Se uma linha for realmente irrecuperável, registre o motivo, trate com a equipe responsável e deixe rastro de quem decidiu e quando.

## Observação e diagnóstico

O que olhar quando alguém disser que uma entrada ou um dado de instrumento não apareceu em outro lugar:

- Primeiro, se a mudança chegou ao banco. Se não está lá, o problema é anterior ao relay.
- Depois, se a linha correspondente está no outbox e em que estado. Pendente há muito tempo aponta para o relay, o broker ou uma trava.
- Se já foi marcada como entregue, o problema está depois: fila, consumidor, ou um consumidor que descartou a mensagem por ser repetida.
- Para dado de instrumento, confira também se o objeto existe no Azure Blob Storage e se a referência na mensagem está correta.

Sinais de saúde que vale ter em painel, em termos gerais: idade da linha pendente mais antiga, quantidade de linhas pendentes, taxa de publicação, erros de conexão com o broker e erros de acesso ao banco. A idade da linha mais antiga é a mais útil, porque um backlog grande que anda é normal e um pequeno que não anda é problema.

Nos logs, procure a identificação da linha e da entrada de caderno, para poder seguir uma mudança do começo ao fim. Se a correlação não aparecer nos logs, é lacuna a corrigir, não algo a contornar.

## Notas para quem mexe no código

O projeto do relay é um aplicativo em C# sobre .NET, que fala com SQL Server e RabbitMQ. O modo normal roda como serviço contínuo; o modo de drenagem é o mesmo código com uma condição de término. Ao alterar a varredura, mantenha os dois modos coerentes: um bug que só aparece na drenagem costuma aparecer na janela de manutenção, na pior hora.

Regras que tento respeitar ao mexer:

- Não mudar a ordem de publicação por entrada sem revisar o impacto na trilha de auditoria.
- Não marcar como entregue antes da confirmação do broker.
- Manter a leitura em lotes limitados, para não segurar trava no banco por muito tempo.
- Qualquer novo tipo de mensagem precisa ter consumidor idempotente definido antes de ir para produção.
- Testar a drenagem contra um banco com linhas pendentes de verdade, incluindo uma linha que falha, e não só contra o caso feliz.

Se a mudança afetar o formato da mensagem, avise quem consome. Compliance depende do histórico, e mensagens antigas ainda pendentes precisam continuar legíveis depois da atualização.

## Pendências e dúvidas em aberto

Coisas que não estão confirmadas nesta nota e que convém verificar no código ou com a equipe antes de tratar como certas:

- Como exatamente o relay trata a linha que falha sempre: pular, parar ou isolar.
- Se há suporte seguro a mais de uma instância simultânea e com que garantia de ordem.
- Se a drenagem aceita outras opções além da que está no comando acima, e qual o código de saída quando sobram linhas.
- Qual a política de retenção das linhas já entregues no outbox e quem limpa.

Quando alguma dessas for esclarecida, atualize esta nota em vez de criar outra sobre o mesmo assunto.
