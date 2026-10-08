---
id: 01KXYYZHT4S9XJAY7M50B4PYHS
created: 2026-07-20T02:11-03:00
---

# ledger-matcher: chave composta de casamento (processor_ref + amount + currency)

Decisão: o ledger-matcher casa um registro do arquivo de liquidação da processadora com um lançamento do ledger interno usando a chave composta `processor_ref` mais `amount` mais `currency`. Antes a ideia era casar só por valor (`amount`). Isso não serve, porque valores idênticos aparecem o tempo todo dentro de um mesmo lote de um merchant. Este registro existe para ninguém reabrir a discussão sem saber por que a chave simples foi descartada.

Resumindo para quem só vai ler o começo: o ledger-matcher não casa mais só por valor. A chave é `processor_ref` + `amount` + `currency`, e um par só é considerado casado quando os três campos batem. O motivo é que valores repetidos dentro de um lote do mesmo merchant geram casamentos ambíguos e falsos.

## Contexto

O Ledgerlark reconcilia arquivos de liquidação de processadoras de cartão contra os lançamentos do ledger interno e sinaliza divergências para revisão. Quem usa são times de operações financeiras de marketplaces. O ledger-matcher é o componente que decide, para cada linha do arquivo da processadora, qual lançamento interno corresponde a ela, ou se não há correspondência.

Num marketplace, um mesmo merchant vende muitos itens parecidos. Preços redondos, taxas fixas, assinaturas e promoções fazem com que vários pagamentos do mesmo lote tenham exatamente o mesmo valor. Quando o lote é liquidado, a processadora devolve todas essas linhas, e o ledger interno tem lançamentos com valores iguais também. Casar só por valor, nessa situação, não identifica nada: existem várias linhas candidatas dos dois lados e nenhuma razão para preferir uma sobre a outra.

O fluxo de dados, em linhas gerais: os arquivos e eventos chegam por Kafka, o ledger-matcher consome, consulta e grava no PostgreSQL, e os resultados e divergências saem por gRPC para a camada de revisão. A infraestrutura é provisionada com Terraform. Nada disso muda com esta decisão; só muda a forma de montar a chave de comparação dentro do ledger-matcher.

## O problema com casar só por valor

Com a chave simples, o comportamento observado era de dois tipos de erro, ambos ruins para quem faz conciliação.

O primeiro é o casamento cruzado. Duas linhas da processadora com o mesmo valor e dois lançamentos internos com o mesmo valor: o matcher pareia na ordem em que as coisas aparecem. Se a ordem de um lado difere da ordem do outro, o par fica trocado. Os totais fecham, então ninguém percebe, mas cada lançamento está ligado à transação errada. Isso estraga rastreabilidade, estorno e disputa (chargeback), porque na hora de investigar o analista segue o vínculo e chega na transação errada.

O segundo é a divergência falsa ou escondida. Se falta uma linha de um lado, o matcher casa com a vizinha de mesmo valor e acusa a divergência na linha errada, ou não acusa nada porque sobra outra de valor igual. O analista recebe um alerta apontando para o item errado, ou não recebe alerta nenhum para um item que realmente está sem correspondência.

Nenhum desses erros é raro. Quanto maior o merchant e mais padronizado o catálogo, mais frequente o problema. Por isso a decisão não é uma otimização: é uma correção de corretude.

## A decisão

A chave de casamento passa a ser composta por três campos:

- `processor_ref`: a referência que a processadora atribui à transação. É o campo que realmente distingue uma linha da outra quando o valor se repete.
- `amount`: o valor da transação.
- `currency`: a moeda do valor.

Um par (linha da processadora, lançamento interno) só é casado quando os três coincidem. Se o `processor_ref` bate mas `amount` ou `currency` diferem, não é um casamento: é uma divergência que vai para revisão, e o motivo precisa dizer qual campo diferiu.

```text
chave de casamento = processor_ref + amount + currency
```

O `processor_ref` é o que dá unicidade na prática. `amount` e `currency` continuam na chave por um motivo diferente: servem de verificação. Se a referência encontra um lançamento mas o valor ou a moeda estão diferentes, isso é justamente o tipo de mismatch que o Ledgerlark existe para achar. Se tirássemos esses dois campos da chave e comparássemos só o `processor_ref`, a divergência de valor deixaria de ser um não-casamento e teria que ser tratada como um segundo passo separado. Manter tudo na chave deixa a regra única e fácil de explicar.

## Por que `currency` entra na chave

Pode parecer redundante, mas não é. Um mesmo valor numérico em moedas diferentes não é o mesmo valor. Marketplaces que vendem para mais de um país recebem liquidações em mais de uma moeda, e o número do `amount` pode coincidir por acaso entre moedas. Sem `currency` na chave, um pagamento em uma moeda poderia ser casado com um lançamento em outra só porque os números são iguais.

Também há o caso de erro de configuração: lançamento gravado com a moeda errada no ledger interno. Com `currency` na chave, isso aparece como não-casamento e vai para revisão, em vez de passar batido.

## Alternativas consideradas

**Só `amount`.** Descartada pelos motivos acima. É a opção original e é a que causa o problema.

**`amount` mais `currency`, sem `processor_ref`.** Resolve a mistura entre moedas, mas não resolve o problema central, que é a repetição de valores dentro do lote do mesmo merchant. Continua ambíguo.

**Só `processor_ref`.** É quase suficiente para identificar, mas joga fora a verificação de valor e moeda na própria regra de casamento. Quando a referência existe dos dois lados e o valor difere, o matcher teria que casar e depois checar, e o resultado "casado" passaria a significar "referência igual", não "transação igual". Preferimos que casado signifique que tudo bate.

**Casamento aproximado ou por janela de tempo e heurística.** Foi lembrado como forma de desempatar valores iguais usando data ou ordem. Descartado: heurística de desempate é exatamente o que produz o casamento cruzado silencioso. Preferimos uma chave determinística e, quando ela não fecha, mandar para revisão humana.

## Consequências

Positivas: o casamento passa a ser determinístico; a ordem das linhas deixa de importar; divergências apontam para o item certo; o motivo da divergência pode citar o campo que não bateu.

Custos e cuidados:

- O ledger interno precisa guardar o `processor_ref` em cada lançamento que será conciliado. Lançamento sem a referência não consegue casar e vai para revisão. É preciso verificar, produtor por produtor, se a referência chega ao ledger de forma confiável antes de ela chegar ao matcher.
- Se a processadora reenviar ou corrigir uma referência, o lançamento antigo pode ficar sem par. Isso deve ser tratado como divergência visível, não escondido.
- Índices no PostgreSQL precisam cobrir a chave composta, na mesma ordem em que o ledger-matcher consulta, para a busca não virar varredura quando os lotes forem grandes. Ajustar o índice é parte da implementação, não opcional.
- Duplicatas legítimas da mesma chave completa (mesmo `processor_ref`, mesmo `amount`, mesma `currency`) não são esperadas. Se aparecerem, o matcher não deve escolher uma ao acaso: deve sinalizar para revisão como duplicidade.

## Pontos em aberto

Não está decidido ainda como tratar processadoras que não fornecem uma referência estável por transação. Para elas a chave composta perde o campo que desempata, e será preciso uma decisão separada, provavelmente um identificador derivado ou um fluxo de revisão mais pesado. Isso não muda a decisão aqui: para as processadoras que fornecem o `processor_ref`, a chave é essa.

Também falta registrar, quando o time fechar, a regra exata para lançamentos que chegam depois da liquidação. Hoje a leitura é que ficam pendentes até a referência aparecer dos dois lados, e só depois entram no casamento.

## Como verificar

Para conferir que a decisão está sendo respeitada, montar um lote de teste de um único merchant com várias linhas de valor idêntico e referências diferentes, e embaralhar a ordem de um dos lados. O resultado esperado é que todos os pares sejam os mesmos, independente da ordem. Depois, alterar o `amount` de uma linha mantendo o `processor_ref`, e alterar a `currency` de outra: as duas devem aparecer como divergência, cada uma com o campo correto indicado. Por fim, remover uma linha de um dos lados e confirmar que a divergência aponta para a linha removida, e não para uma vizinha de mesmo valor.

Se algum desses testes falhar, o primeiro suspeito é a montagem da chave dentro do ledger-matcher, em particular se algum dos três campos está sendo normalizado de forma diferente nos dois lados antes da comparação.
