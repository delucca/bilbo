---
id: 01KWDGKEWGX9KQQPWNARWZ85S0
created: 2026-06-30T21:17-03:00
---

# keyvane-rotator: período de carência da versão anterior do segredo

Decisão: o keyvane-rotator mantém a versão anterior do segredo viva por `grace_period` de `3600 seconds` depois de cada rotação. Ou seja, depois que uma nova versão é emitida, a versão antiga continua válida durante `3600 seconds` (uma hora) e só então é revogada. O motivo é que os consumidores conseguem pegar a nova versão sem precisar de redeploy.

## Decisão

O valor de `grace_period` é `3600 seconds`. Esse intervalo começa a contar no momento da rotação e vale para a versão que acabou de ser substituída. Durante esse tempo existem duas versões válidas ao mesmo tempo: a nova e a anterior. Passado o prazo, a anterior deixa de valer.

## Por que

O Keyvane existe para rotacionar segredos sem redeploy. Se o keyvane-rotator revogasse a versão antiga no instante da rotação, qualquer serviço que ainda segurasse a versão antiga começaria a falhar antes de conseguir buscar a nova. Com a janela de `3600 seconds`, cada consumidor tem tempo de recarregar o segredo por conta própria, no ritmo dele, e o serviço não precisa ser reiniciado nem reimplantado.

## O que isso significa para quem consome

- Os consumidores devem buscar a versão nova dentro da janela de `grace_period`, ou seja, em até `3600 seconds` depois da rotação.
- Um consumidor que perder a janela vai ficar com um segredo já revogado e vai falhar na autenticação até recarregar.
- Não é preciso coordenar a rotação com o ciclo de deploy das equipes de aplicação.

## Efeito na segurança

A versão anterior fica válida por mais tempo do que ficaria sem a carência. Se um segredo vazou, a rotação não invalida a cópia vazada de imediato: ela continua útil até o fim de `grace_period`. Para uma revogação de emergência, a versão antiga precisa ser revogada à mão, sem esperar a janela. A comunicação entre os componentes continua protegida por mTLS com identidades SPIFFE, e isso não muda com a carência.

## Onde mexer

O valor é configuração do keyvane-rotator, sob o nome `grace_period`. Alterar o número muda a janela de todas as rotações seguintes. Antes de reduzir, confirme com as equipes de aplicação que os consumidores recarregam o segredo mais rápido do que o novo prazo.

## Pontos de atenção

- Ao reduzir `grace_period`, o risco é derrubar consumidores lentos.
- Ao aumentar, o risco é deixar segredos antigos válidos por mais tempo.
- Se uma rotação ocorrer de novo dentro da janela, é preciso conferir quantas versões antigas ficam vivas ao mesmo tempo.

## Resumo rápido

O keyvane-rotator mantém a versão anterior do segredo viva por `grace_period` de `3600 seconds` após a rotação, porque assim os consumidores pegam a nova versão sem redeploy.
