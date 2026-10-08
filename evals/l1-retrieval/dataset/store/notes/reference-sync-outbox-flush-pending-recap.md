---
id: 01JYX30PST9Q647484PDG3S800
created: 2025-06-29T02:37-03:00
---

# sync-outbox-relay: flush de pendentes (anotação solta)

Anotação rápida sobre como o sync-outbox-relay esvazia as mensagens pendentes da outbox. Escrevi de cabeça, sem conferir tudo no código, então trate como parcial. O lado da publicação no broker está em [[sync-outbox-relay]]-publisher, ou melhor, na nota [[sync-outbox-relay-publisher]]; aqui fica só o flush.

## Contexto

O sync-outbox-relay lê linhas da tabela de outbox no SQL Server e entrega cada uma ao RabbitMQ. Cada linha representa uma mudança no caderno eletrônico ou uma leitura de instrumento que precisa chegar ao outro lado e entrar na trilha de auditoria. Enquanto a linha não foi confirmada, ela fica como pendente.

## O que é "pendente"

Pendente é a linha gravada na mesma transação da alteração do caderno, mas ainda sem marca de publicada. Não é erro. É o estado normal por alguns instantes depois de qualquer gravação.

## Como o flush roda

Um laço em segundo plano acorda num intervalo configurado, pega um lote de pendentes na ordem de criação e tenta publicar. O tamanho do lote é o limite configurado, não o valor que eu lembro de ter visto. Se o lote vier cheio, o laço repete sem dormir.

## Ordem das mensagens

A ordem por entrada do caderno importa para a auditoria. Por isso o relay não paraleliza dentro da mesma entrada. Entre entradas diferentes pode haver paralelismo, mas confirmei isso só de leitura rápida.

## Confirmação e marca de publicada

A linha só ganha a marca depois que o broker confirma o recebimento. Se o processo cair entre a publicação e a marca, a mensagem sai de novo no próximo ciclo. Quem consome precisa tolerar duplicata.

## Falhas no broker

Se o RabbitMQ estiver indisponível, o flush falha e as linhas continuam pendentes. Há retentativa com espera crescente até um teto configurado. Depois disso o laço continua tentando no ritmo normal, sem descartar nada.

## Anexos no Blob Storage

Mensagens que apontam para arquivos de instrumento só carregam a referência. O arquivo já deve estar no Azure Blob Storage antes de a linha entrar na outbox. Se a referência chegar antes do blob, o consumidor reclama, e a causa não está no flush.

## Sinais de acúmulo

Para saber se o flush está atrasado, olhar a idade da linha pendente mais antiga, não a contagem. Uma contagem alta com idade baixa é só rajada. Idade alta é problema de verdade, e normalmente indica broker fora ou lote travado.

## Lote travado

Já vi uma linha problemática bloquear o lote inteiro porque a ordem exige que ela saia primeiro. Falta confirmar se existe um desvio para linhas envenenadas. Não sei se há tabela de mortas.

## Concorrência entre instâncias

Com mais de uma instância do relay, o acesso às linhas usa bloqueio no SQL Server para uma instância não pegar o lote da outra. Não anotei o mecanismo exato, vale reler antes de mexer.

## Limpeza

Linhas já publicadas são removidas ou arquivadas por uma rotina separada, com retenção configurada. Compliance pode pedir retenção maior, então não encurtar sem avisar.

## Pendências desta nota

Falta conferir o comportamento de desligamento: se o flush termina o lote atual ou abandona no meio. Falta também checar métricas expostas e onde ficam os alertas.

## Veja também

A parte de publicação, confirmações do broker e roteamento está na nota do publisher já citada acima.
