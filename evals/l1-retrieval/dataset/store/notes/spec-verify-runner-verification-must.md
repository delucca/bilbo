---
id: 01KHTM2H1GJTDMJ7P81GDHEKVA
created: 2026-02-19T06:35-03:00
---

# Especificação do verify-runner: limite de tempo por job

O `verify-runner` executa os testes direcionados que verificam cada pull request de atualização de dependência aberto pelo PatchPilot. Esta nota fixa o limite de tempo de cada job de verificação e o que se espera dele quando o limite estoura.

## Regra principal

Cada job de verificação do `verify-runner` deve terminar dentro do limite `timeout-minutes: 20`. Passou disso, o job é cancelado e a verificação conta como falha, não como sucesso parcial.

## Onde o limite vale

O limite vale por job, não por pull request nem por repositório. Um PR que dispara vários jobs de verificação dá a cada um deles a sua própria janela. O tempo de espera na fila do GitHub Actions não deve ser contado como execução do job.

## Como configurar

No workflow do GitHub Actions, a chave fica no nível do job:

```yaml
jobs:
  verify:
    timeout-minutes: 20
```

## Por que existe

Plataformas com muitos repositórios não podem deixar um job travado ocupar runners por horas. Um teste pendurado, uma rede que não responde ou um container que não sobe precisam aparecer como falha rápida, para o engenheiro de plataforma agir.

## O que conta dentro do limite

Entra tudo o que o job faz: preparar o ambiente, subir o container Docker, instalar dependências, rodar os testes direcionados e gravar o resultado no SQLite. Se o preparo consome muito do tempo, sobra menos para os testes.

## Comportamento no estouro

Ao estourar, o job é marcado como falho por tempo esgotado. O PR de atualização não deve ser aprovado nem mesclado automaticamente nesse estado. O motivo registrado deve deixar claro que foi timeout, e não falha de teste.

## Reexecução

Uma nova tentativa é permitida, mas não deve aumentar o limite de forma automática. Se o mesmo PR estourar de novo, trate como problema do repositório alvo ou da seleção de testes, e não como azar da infraestrutura.

## Alterar o limite

Subir ou baixar o valor é uma decisão de projeto. Se mudar, atualize esta nota e o workflow juntos, para os dois não divergirem.

## Testes direcionados

A seleção dos testes deve caber na janela. Se uma seleção costuma chegar perto do limite, reduza o escopo dos testes em vez de esticar o tempo.

## Observabilidade

Vale registrar a duração de cada job, para ver quais repositórios chegam perto do limite antes de estourar. Isso ajuda a achar jobs lentos cedo.

## Pontos em aberto

Ainda não está definido se jobs que estouram devem gerar alerta separado para os engenheiros de plataforma. Por enquanto fica só o registro da falha.

## Resumo rápido

Um job de verificação, uma janela fixa, falha clara quando acaba o tempo.
