---
id: 01KQM7YW211GBRXFQFYFXKH6M2
created: 2026-05-02T08:42-03:00
---

# ledgerlark-infra: tempo e tamanho do plan de produção

Nota rápida sobre o terraform plan de produção do ledgerlark-infra. O número que importa: o plan do ambiente de produção gerencia 214 resources e leva 3 min 20 s para terminar. Quem for rodar isso no meio de um incidente ou numa revisão de PR deve contar com esse tempo e não achar que o plan travou.

## Contexto

O ledgerlark-infra é o repositório de infraestrutura do Ledgerlark, o sistema que concilia arquivos de liquidação das processadoras de cartão com os lançamentos do ledger interno e marca divergências para revisão. Os times de operações financeiras dos marketplaces dependem dele estar de pé, então mudanças de infra em produção passam por plan antes de qualquer apply.

## O número em uma frase

Plan de produção do ledgerlark-infra: 214 resources, 3 min 20 s. Se alguém perguntar quanto tempo demora ou quantos recursos o Terraform gerencia em produção, é isso.

## O que entra nos 214 resources

A contagem cobre tudo que o state de produção do ledgerlark-infra gerencia. Em termos gerais, são recursos de rede, banco PostgreSQL, cluster e tópicos do Apache Kafka, serviços Go expostos por gRPC, permissões e monitoramento. Não detalhei a divisão por tipo aqui; vale gerar essa lista direto do state quando for preciso.

## Por que o tempo importa

Com 3 min 20 s por plan, o ciclo de feedback em PR não é instantâneo. Se o plan rodar a cada push, o custo soma rápido. Para mudanças pequenas, dá para pensar em plan com alvo restrito, mas isso esconde dependências, então só em investigação, nunca como plano final.

## Onde o tempo provavelmente vai

Quase todo o tempo é refresh do state: o Terraform consulta a API do provedor para cada um dos recursos antes de calcular o diff. Quanto mais recursos, mais chamadas. Isso é hipótese razoável, não medi por recurso.

## Como interpretar variações

Se o plan passar bem acima de 3 min 20 s, desconfie primeiro de throttling na API do provedor ou de lock de state preso. Se ficar bem abaixo, confira se o plan está mesmo apontando para produção e não para outro ambiente menor.

## Como medir de novo

Rodar o plan de produção com cronômetro simples em volta, na mesma máquina ou no mesmo runner de CI, e comparar. Anotar a contagem de recursos que o plan imprime. Se a contagem divergir de 214 resources, atualizar esta nota, porque a base mudou.

## Limites do que eu sei

O tempo de 3 min 20 s é uma medida de produção; não registrei a variância entre execuções nem a condição da rede na hora. Trate como ordem de grandeza confiável, não como SLA.

## Riscos de um plan lento

Plan demorado tenta o time a pular a etapa ou a aplicar com um plan antigo. Aplicar com plan velho em produção é o risco real, porque o state pode ter mudado no meio tempo. Sempre gerar o plan perto do apply.

## Lock de state

Enquanto o plan roda, ele pode segurar o lock do state, dependendo do backend. Duas pessoas planejando produção ao mesmo tempo podem esbarrar uma na outra. Combinar antes no canal do time evita espera inútil.

## Relação com os outros ambientes

Ambientes menores terão menos recursos e plans mais rápidos. Não extrapole o tempo de produção para eles, nem o contrário. Esta nota trata só de produção.

## Possíveis melhorias

Dividir o state em partes menores (rede, dados, serviços) reduziria o tempo de cada plan e o raio de impacto de um erro. Também ajuda aumentar o paralelismo do Terraform, com cuidado com limites de API. Nada disso foi feito; são ideias.

## Custos de dividir o state

Separar o state exige mover recursos, tratar dependências entre as partes e ajustar o CI. Para um conjunto de 214 resources talvez compense, talvez não. Decidir com dados de dor real do time.

## Quando reavaliar

Reavaliar se o plan passar a incomodar no dia a dia, se a contagem de recursos crescer bastante ou se o provedor começar a limitar chamadas. Até lá, o custo atual é aceitável.

## Pendências

- Registrar a variância do tempo em algumas execuções.
- Listar a contagem por tipo de recurso a partir do state.
- Verificar se o CI faz refresh desnecessário em PRs que não tocam produção.

## Resumo para quem chegou agora

O ledgerlark-infra tem 214 resources em produção, o plan leva 3 min 20 s, o gargalo provável é o refresh, e o risco principal é aplicar um plan velho. O resto são ideias ainda sem decisão.
