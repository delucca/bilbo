---
id: 01M05S6QGYG3A62QZ5YZR16MV0
created: 2026-08-16T14:16-03:00
---

# opa-eval-lambda: opções consideradas

Anotação rápida sobre as opções gerais que apareceram para o `opa-eval-lambda`, o componente que avalia as configurações de nuvem contra as políticas do AuditMesh. Nada aqui é decisão fechada. É um levantamento para não refazer a conversa depois.

## Contexto

O `opa-eval-lambda` recebe uma configuração de infraestrutura, roda as políticas escritas em Rego e devolve as violações. Quem consome o resultado é a parte que grava no DynamoDB e abre tickets no Jira. O que importa aqui é como empacotar e executar o motor de políticas dentro de uma função Lambda.

## Opção A: OPA como binário empacotado

Colocar o binário do OPA junto com a função e chamá-lo por subprocesso a partir do Python. É simples e usa o motor oficial, sem surpresas de comportamento.

### Pontos a favor

Os bundles de política ficam no formato normal do OPA. Testar localmente é igual a testar em produção.

### Pontos contra

O custo de iniciar um processo a cada avaliação pesa em cold start. Também é preciso cuidar da arquitetura do binário e do tamanho do pacote.

## Opção B: OPA como servidor local na própria função

Subir o OPA em modo servidor dentro do ambiente de execução e falar com ele por HTTP local. Reaproveita o processo entre invocações quentes.

### Pontos a favor

Evita o custo de subir processo a cada chamada. Dá acesso à API de decisão completa.

### Pontos contra

O ciclo de vida do processo dentro do Lambda é incômodo: precisa esperar ficar pronto e tratar falhas silenciosas. Complica o encerramento.

## Opção C: Rego compilado para WebAssembly

Compilar as políticas para Wasm e executar com um runtime em Python. Não precisa de processo separado.

### Pontos a favor

Isolamento bom e inicialização rápida. A política vira um artefato versionável.

### Pontos contra

Nem todos os recursos do Rego e dos builtins têm o mesmo suporte. O suporte do lado Python precisa ser avaliado com cuidado antes de qualquer compromisso.

## Opção D: OPA em serviço separado

Tirar a avaliação do Lambda e chamar um serviço OPA dedicado. Fica fora do escopo do componente, mas foi citado.

### Observações

Adiciona rede, autenticação e operação de outra peça. Só faz sentido se o volume justificar.

## Distribuição das políticas

Há três caminhos gerais: embutir os bundles no pacote da função, baixar de um armazenamento de objetos na inicialização, ou usar uma camada (layer) separada. Embutir é o mais previsível; baixar permite atualizar sem novo deploy, ao custo de uma dependência a mais na partida.

## Entrada e normalização

As configurações chegam em formatos variados. Vale decidir se a normalização acontece antes do componente ou dentro dele. Dentro, o Rego fica mais complexo; fora, o contrato de entrada precisa ser bem definido.

## Saída e integração

O formato das violações deve ser estável, porque o gerador de tickets e o registro no DynamoDB dependem dele. Falta definir como lidar com reavaliações da mesma violação e evitar tickets duplicados.

## Testes

Testes de política com o próprio framework de testes do OPA, mais testes em Python para o código de borda. Casos reais anonimizados ajudam mais que exemplos inventados.

## Riscos em aberto

Cold start, tamanho do pacote, limite de tempo de execução em lotes grandes e consumo de memória com políticas pesadas. Nenhum desses foi medido ainda.

## Próximos passos possíveis

Montar uma comparação pequena entre as opções A e C com um conjunto representativo de políticas, e só então escolher.
