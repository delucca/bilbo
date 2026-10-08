---
id: 01M1M7KSP20Z2SJQ93JYZMVRHY
created: 2026-09-03T15:13-03:00
---

# signature-pad-view: coordenada Gradle consumida

Esta nota substitui a nota anterior "signature pad view consumed" e traz o valor novo: o componente `signature-pad-view` agora é consumido pela coordenada Gradle `com.parcelpin:signature-pad:2.4.0`, no lugar da coordenada 2.3.0.

## Resumo rápido

O `signature-pad-view` é a view Android que o app do entregador usa para capturar a assinatura de quem recebe o pacote. A coordenada atual é `com.parcelpin:signature-pad:2.4.0`. Quem ainda apontar para a 2.3.0 está desatualizado.

## Nome interno

O codinome interno do componente é `squiggl`. Quando aparecer `squiggl` em conversa, commit, branch ou issue antiga, é o mesmo componente que aqui se chama `signature-pad-view`. Nesta nota usamos sempre `signature-pad-view`.

## Coordenada atual

Use `com.parcelpin:signature-pad:2.4.0` na declaração de dependência do módulo Android que mostra a tela de assinatura. Essa coordenada substitui a anterior, a 2.3.0. Não misture as duas no mesmo build.

## O que mudou

Só o consumo mudou: a versão da coordenada subiu da 2.3.0 para a 2.4.0. Esta nota não descreve o conteúdo do release; para isso, olhe o histórico do próprio componente.

## Onde atualizar

Procure a declaração antiga nos arquivos de build do app e de qualquer módulo que dependa da view. Se houver catálogo de versões, troque lá. Se a versão estiver fixa num build script, troque no script.

## Como conferir

Depois de trocar, sincronize o projeto no Android Studio e rode o build normal. Confira também a árvore de dependências para ver se nenhuma biblioteca puxa a 2.3.0 de forma transitiva.

## Armadilhas

Cache do Gradle pode manter a versão velha em builds locais; limpe o cache do módulo se a resolução parecer errada. Outro risco é um módulo esquecido ainda apontando para a 2.3.0, o que gera duas versões no classpath.

## Relação com o fluxo offline

A captura de assinatura faz parte da prova de entrega, que precisa funcionar sem rede. A troca de coordenada não deve alterar esse comportamento, mas vale testar uma entrega em modo avião depois da atualização.

## Relação com Firestore e Protobuf

A view só captura o traço. O que é gravado no Cloud Firestore e como a assinatura é serializada em Protocol Buffers fica fora do componente. Nada disso foi alterado por esta mudança de versão.

## Testes sugeridos

Desenhar uma assinatura, limpar e desenhar de novo. Girar a tela no meio do traço. Salvar sem rede e sincronizar depois. Em geral, esses três casos pegam a maioria das regressões.

## Quem consome

O app de entregadores é o consumidor principal. Se outro módulo interno usar a view, ele também deve migrar para a coordenada atual.

## Rollback

Se a nova versão causar problema, voltar para a 2.3.0 é possível trocando a coordenada de volta. Registre o motivo numa nota própria, e não nesta, para não misturar referência com incidente.

## Perguntas frequentes

Qual é a coordenada atual do `signature-pad-view`? `com.parcelpin:signature-pad:2.4.0`. Qual é o codinome? `squiggl`. O que ela substitui? A coordenada 2.3.0.

## Pendências

Confirmar que todos os módulos migraram. Confirmar que nenhuma dependência transitiva ainda traz a versão antiga.

## Histórico desta nota

Substitui a nota anterior sobre o consumo do componente. Se a coordenada mudar de novo, atualize esta nota em vez de criar outra.
