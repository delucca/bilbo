---
id: 01K07872XAC28TK3TFXDC1NF8Y
created: 2025-07-15T11:35-03:00
---

# fx-rate-loader: onde ficam as peças

Nota rápida de referência sobre o `fx-rate-loader`, o componente do Ledgerlark que traz taxas de câmbio para dentro do sistema. A reconciliação precisa delas quando o arquivo de liquidação da processadora vem numa moeda e o lançamento do razão interno está em outra. Aqui só aponto onde cada coisa mora, sem detalhes finos. Para o contexto de quem revisa os desvios depois, veja [[ledger-entries-review-themes]].

## Visão geral

O `fx-rate-loader` é um serviço em Go, separado do resto da reconciliação. Ele busca taxas em uma fonte externa, valida o que chegou, grava no PostgreSQL e avisa os outros serviços por Kafka. Quem precisa de uma taxa na hora consulta por gRPC. Não faz reconciliação nenhuma: só fornece o dado.

## Código Go

O ponto de entrada fica na pasta de comandos do repositório do serviço, como nos outros binários Go do projeto. A lógica fica em pacotes internos, divididos mais ou menos assim: cliente da fonte de taxas, validação e normalização, camada de persistência e publicação de eventos. Se for mexer em algo, comece pelo pacote do cliente da fonte, que é onde a maioria dos problemas aparece.

## Fonte das taxas

A fonte é um provedor externo, acessado por HTTP. As credenciais não ficam no código; vêm da configuração do ambiente e do gerenciador de segredos que o Terraform provisiona. O formato da resposta muda de provedor para provedor, então o parsing está isolado em um só lugar para facilitar a troca.

## Banco de dados PostgreSQL

As taxas ficam em tabelas próprias do serviço, com o par de moedas e a data de referência como chave lógica. As migrações moram junto do código do serviço, em diretório próprio. Outros serviços não devem ler essas tabelas direto; o caminho é o gRPC. Vale conferir como a carga trata taxa repetida para o mesmo dia, porque isso já gerou dúvida antes.

## Kafka

Depois de gravar um lote, o loader publica um evento em um tópico dedicado a atualizações de taxa. Os consumidores principais são os serviços de reconciliação, que invalidam cache quando o evento chega. Definições de tópico e grupos de consumo ficam na infraestrutura, não no código do serviço.

## Interface gRPC

Os arquivos de definição do contrato ficam no repositório compartilhado de protos, e o código gerado é versionado junto. A consulta típica pede a taxa de um par de moedas para uma data. Mudanças no contrato precisam ser compatíveis com os clientes existentes, então confira quem consome antes de alterar.

## Terraform

A infraestrutura do serviço (execução agendada ou contínua, acesso ao banco, tópicos, segredos e permissões) é descrita em módulos Terraform no repositório de infraestrutura. Cada ambiente tem sua variação. Mudança de agendamento da carga passa por lá, não pelo código Go.

## Operação e monitoramento

Os logs do serviço seguem o padrão dos outros componentes Go. Falha na carga normalmente aparece primeiro como ausência de evento no Kafka e depois como reclamação de reconciliação com taxa faltando. Alertas e painéis ficam na ferramenta de observabilidade da equipe; procure pelo nome do componente.

## Pendências

Falta documentar melhor o que acontece em feriados e fins de semana, quando a fonte não publica taxa nova. Também falta registrar quem é o responsável pelo contato com o provedor. Atualizar esta nota quando isso for esclarecido.
