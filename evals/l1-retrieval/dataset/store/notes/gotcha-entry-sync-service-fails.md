---
id: 01KE6DKDG88T4A79DPNB21D5XP
created: 2026-01-05T03:30-03:00
---

# entry-sync-service quebra com notebook sem revisões

O entry-sync-service cai com `System.InvalidOperationException: Sequence contains no elements` quando um notebook tem zero revisões e alguém chama `First()` no histórico dele. Não é um bug de rede nem de fila: é uma suposição errada no código de que todo notebook já tem pelo menos uma revisão. Anotei aqui porque é fácil perder tempo olhando RabbitMQ, SQL Server ou Azure Blob Storage antes de chegar na causa real.

## Sintoma

O serviço lança a exceção durante a sincronização de um notebook específico. O texto completo da exceção é:

```
System.InvalidOperationException: Sequence contains no elements
```

A mensagem é genérica do LINQ e não diz qual notebook causou o problema. O stack trace aponta para a chamada de `First()` sobre a coleção de revisões, e é isso que denuncia o caso. Se você só vê a mensagem em um log resumido, ela parece um erro qualquer de coleção vazia.

Efeitos que notei ou que dá para esperar:

- A sincronização daquele notebook não termina.
- A mensagem que disparou o processamento pode voltar para a fila e ser reentregue, repetindo o erro em loop até alguém tratar.
- Outros notebooks podem ficar atrasados se o consumidor ficar preso reprocessando a mesma mensagem.
- Para quem cuida de conformidade, um notebook sem sincronizar significa trilha de auditoria incompleta naquele período, o que precisa ser registrado.

## Causa

O código pega a primeira revisão do histórico com `First()`. Para um notebook com zero revisões, o histórico é uma sequência vazia, e `First()` lança exceção em sequência vazia em vez de devolver nulo. Um notebook assim existe, por exemplo, quando foi criado mas ninguém salvou conteúdo ainda, ou quando a criação foi registrada e a primeira revisão ainda não chegou do instrumento. A ordem dos eventos entre instrumento, fila e banco não é garantida, então o caso aparece de vez em quando e não de forma previsível.

Isso passa despercebido em testes porque os dados de teste quase sempre já vêm com ao menos uma revisão.

## Como corrigir

A correção mínima é trocar `First()` por `FirstOrDefault()` e tratar o resultado nulo de forma explícita. O que fazer no caso nulo é decisão de negócio, não só de código:

- Pular o notebook neste ciclo e tentar de novo depois, registrando um log claro com o identificador do notebook.
- Tratar como notebook novo, sem base de comparação, se a regra de sincronização permitir.
- Nunca criar uma revisão fictícia só para evitar o erro, porque isso contamina a trilha de auditoria.

Também vale checar os outros pontos do entry-sync-service que usam `First()`, `Last()` ou `Single()` sobre históricos, já que o mesmo raciocínio errado pode estar repetido. Prefira variantes `OrDefault` quando a coleção puder legitimamente estar vazia.

## Como reproduzir e verificar

1. Crie ou escolha um notebook de teste sem nenhuma revisão no SQL Server.
2. Dispare a sincronização desse notebook pelo caminho normal, publicando a mensagem na fila do RabbitMQ.
3. Confirme que a exceção aparece nos logs do entry-sync-service com o texto acima.
4. Depois da correção, repita e confirme que o serviço registra o caso, não lança exceção e segue para o próximo notebook.

Adicione um teste automatizado com histórico vazio. Esse é o caso que faltava na suíte, e sem ele a regressão volta fácil.

## Observações para quem for mexer nisso

- Não esconda o erro com um `catch` genérico em volta do loop. Isso apaga o sinal e deixa notebooks sem sincronizar sem ninguém saber.
- Se a mensagem ficar reentregando, ela precisa ir para uma fila de mensagens mortas depois de algumas tentativas, senão trava o consumidor.
- Avise o pessoal de conformidade se houver notebooks que ficaram sem sincronizar por causa disso, porque a trilha de auditoria deles teve lacuna.
- Quando o notebook ganhar a primeira revisão, a sincronização seguinte deve pegá-la normalmente. Vale confirmar isso no teste.
