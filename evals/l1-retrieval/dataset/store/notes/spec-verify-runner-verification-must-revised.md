---
id: 01M2SMRZHH9NCT1K367BAFVEPV
created: 2026-09-18T03:55-03:00
---

# verify-runner: limite de tempo dos jobs de verificação

Esta nota substitui a nota anterior sobre "verify runner verification must". O valor novo é `timeout-minutes: 30` para os jobs de verificação do `verify-runner`, no lugar do limite antigo de 20 minutos.

## Resumo da mudança

O `verify-runner` é a parte do PatchPilot que pega um pull request de upgrade de dependência e roda os testes direcionados para ver se a atualização quebra algo. Cada execução desse tipo é um job de verificação. Até agora o teto de cada job era de 20 minutos. Agora o teto é `timeout-minutes: 30`.

O limite vale para o job de verificação inteiro, não para um teste isolado. Se o job passar do teto, o GitHub Actions cancela a execução e o pull request fica sem resultado de verificação conclusivo. Quem lê o resultado deve tratar esse caso como "não verificado", nunca como "passou".

A mudança não altera quais testes são escolhidos nem a ordem em que rodam. Só muda quanto tempo o job tem para terminar.

## Por que o limite subiu

O limite antigo estava curto para repositórios maiores. Os upgrades que mexem em bibliotecas muito usadas fazem o seletor de testes escolher muitos arquivos, e a fase de instalação de dependências dentro do container também pesa. Em repositórios grandes, o job batia no teto mesmo quando nada estava errado, e a equipe de plataforma tinha que reexecutar na mão.

O custo de subir o teto é que um job travado ocupa o runner por mais tempo antes de ser cancelado. Aceitamos isso porque os travamentos de verdade são raros perto dos falsos timeouts que o limite antigo causava.

Se os timeouts continuarem aparecendo com o valor novo, a primeira coisa a olhar é se o seletor de testes está escolhendo demais, e não subir o teto de novo.

## Onde o valor é aplicado

O valor fica na definição do job de verificação no workflow do GitHub Actions. O trecho abaixo mostra só a chave que importa; o resto do job não muda.

```yaml
jobs:
  verify:
    timeout-minutes: 30
```

Pontos que já deram confusão:

- O `timeout-minutes` do job é o teto do GitHub Actions. Ele corta o job por fora, sem dar chance de o `verify-runner` limpar o estado.
- Qualquer limite interno do `verify-runner` para etapas individuais tem que ficar abaixo do teto do job. Se ficar igual ou acima, o corte externo vence e a mensagem de erro some.
- Se existir outro lugar com o valor antigo copiado (documentação, exemplos, templates de workflow para repositórios clientes), ele deve ser atualizado para `timeout-minutes: 30`.

## Efeito sobre o Docker e o SQLite

Os testes rodam dentro de um container Docker. Quando o GitHub Actions cancela o job por timeout, o container é derrubado junto. Isso significa que um job cancelado não deixa processo de teste vivo, mas também não grava um resultado final.

O PatchPilot guarda o histórico de verificações em SQLite. Um job que estourou o tempo pode deixar um registro em estado "em andamento". Esse registro precisa ser tratado como abandonado pela rotina que reconcilia o estado, e não como verificação ainda rodando. Com o teto maior, a janela em que um registro pode estar legitimamente em andamento também ficou maior, então qualquer regra de "abandonado depois de X tempo" tem que usar um prazo maior que o teto do job. Vale conferir essa regra se alguém mexer no reconciliador.

## Como verificar que o novo limite está valendo

Depois de mexer no workflow, o caminho mais rápido é abrir um pull request de upgrade pequeno e olhar a definição do job na execução. A página da execução mostra o limite configurado. Se mostrar o valor antigo, o workflow que está rodando não é o que você editou, o que costuma acontecer quando o repositório cliente usa uma cópia própria do workflow.

Checklist rápido:

- O workflow do repositório cliente herda a definição ou tem cópia local?
- A cópia local já foi atualizada para o valor novo?
- Algum limite interno de etapa ficou acima do teto do job?
- A rotina que reconcilia registros abandonados usa um prazo maior que o teto?

## O que não mudou e o que ainda está em aberto

Não mudou a política de reexecução. Um job cancelado por timeout continua sendo reportado como falha de infraestrutura, separado de falha de teste, para que a equipe não confunda lentidão com regressão causada pela dependência.

Em aberto:

- Decidir se vale ter um limite diferente por tamanho de repositório, em vez de um valor único. Por enquanto fica um valor só, para manter a configuração simples.
- Ver se dá para avisar no pull request, antes do corte, que o job está perto do teto. Hoje o aviso só aparece depois do cancelamento.
- Medir de tempos em tempos quantos jobs terminam perto do teto. Se muitos terminarem perto dele, o problema é de seleção de testes ou de instalação, e subir o limite só adia isso.

Se você achou esta nota por causa da anterior, ignore o valor de 20 minutos que estava lá. O valor que vale é `timeout-minutes: 30`.
