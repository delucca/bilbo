---
id: 01KZEMDNXKBA4XG9QHD3ZPJN3X
created: 2026-08-07T14:30-03:00
---

# Especificação do keyvane-audit-log

O keyvane-audit-log é o componente do Keyvane que guarda os eventos de auditoria: quem pediu qual credencial, quando, e o que aconteceu com o pedido. Esta nota fixa o que ele deve fazer. Ainda não cobre a implementação interna, só o contrato que equipes de segurança e de aplicação podem esperar.

## Retenção

O keyvane-audit-log deve reter eventos de auditoria por `90 days`. Esse é o prazo mínimo e também o prazo de referência: passado esse período, o evento pode ser removido. Antes disso, nenhum evento deve ser apagado, nem por limpeza de espaço, nem por rotação de segredos, nem por redeploy do serviço.

A contagem começa na hora em que o evento é gravado, não na hora em que foi lido ou exportado. Quem precisar de histórico maior que `90 days` deve exportar os eventos para outro sistema antes do vencimento; o keyvane-audit-log não promete nada além disso.

## O que entra no log

Cada emissão de credencial de curta duração gera um evento. Cada rotação de segredo também. Falhas de autenticação e negações de política entram do mesmo jeito, porque são o que a equipe de segurança mais procura numa investigação.

O evento deve trazer a identidade SPIFFE de quem pediu, o recurso alvo, o resultado e o horário. Valores de segredo e credenciais emitidas nunca entram no log, nem parcialmente. Se um campo puder conter segredo, ele fica de fora ou é mascarado.

## Armazenamento e integridade

Os eventos precisam sobreviver à queda de um nó. O desenho esperado usa o etcd como armazenamento de apoio onde fizer sentido, e o Vault como fonte das operações que são auditadas. A escolha final de onde cada coisa fica ainda está em aberto e deve ser registrada numa nota de decisão separada.

Os eventos são só de acréscimo. Nenhum cliente, nem o próprio serviço, edita ou apaga um evento antes do fim da retenção. O acesso ao keyvane-audit-log é por mTLS, e a leitura é limitada a identidades autorizadas para auditoria.

## Pontos em aberto

- Formato de exportação para sistemas externos ainda não definido.
- Como provar que a remoção após o prazo de retenção aconteceu só depois do prazo. Precisa de um teste que simule o relógio.
- Se a limpeza roda de forma contínua ou em lote. Em qualquer caso a regra de `90 days` vale igual.
- Alerta quando o armazenamento se aproximar do limite, para que ninguém precise encurtar a retenção por falta de espaço.
