---
id: 01M2BSBBACQCBNV1KNPV031N8J
created: 2026-09-12T18:46-03:00
---

# qa-queue: lista de moderadores (anotações soltas)

Anotação rápida sobre como o qa-queue trata a lista de moderadores. Não conferi se já existe outra nota sobre isso, então pode haver sobreposição. Escrevi o que lembro do código e do comportamento em eventos grandes, sem os valores exatos.

## O que é a lista de moderadores

O qa-queue mantém, para cada evento, uma lista de pessoas autorizadas a mexer na fila de perguntas: aprovar, rejeitar, fixar, reordenar e marcar como respondida. Quem produz o evento define essa lista antes de abrir a sala, e o gerente de comunidade pode ajustar durante a transmissão. A lista fica no CockroachDB, ligada ao evento, e uma cópia em memória fica no processo Elixir que cuida da fila daquele evento.

Não é a mesma coisa que o papel de produtor. O produtor sempre pode moderar; o moderador só pode o que o papel dele permite dentro da fila. Vale lembrar disso ao debugar uma permissão negada: primeiro ver o papel, depois a lista.

## Como a lista chega ao cliente

O front em Next.js recebe a lista no join do canal Phoenix, junto com o estado inicial da fila. Depois disso, mudanças chegam como eventos pelo WebSocket. O cliente não consulta a lista por HTTP durante a sessão, só no carregamento da página de administração.

Pontos que já me confundiram:

- A tela de moderação mostra quem está online, mas isso vem da presença do canal, não da lista. Um moderador cadastrado e offline aparece só na lista, não na presença.
- Se o moderador abre duas abas, ele aparece uma vez na lista e duas vezes na presença, dependendo de como a chave de presença é montada.
- O cliente esconde os botões de moderação para quem não está na lista, mas isso é só cosmético. A checagem de verdade acontece no servidor, em cada comando.

## Regras de acesso

Cada comando que altera a fila passa por uma checagem no módulo do qa-queue antes de gravar. A checagem olha a cópia em memória primeiro. Se o usuário não estiver lá, ela não vai ao banco; recusa direto. Isso é rápido, mas significa que a cópia em memória precisa estar correta, senão a pessoa fica bloqueada sem motivo.

Existe um limite configurado de moderadores por evento. O valor usual é suficiente para a maioria dos eventos, mas produtores de eventos muito grandes já pediram para subir. Isso é configuração por plano, não constante no código, então a mudança é por conta do plano e não de deploy.

Remover um moderador tem efeito imediato nos comandos novos. Comandos que já estavam em andamento no momento da remoção podem terminar, porque a checagem acontece na entrada.

## Sincronização e consistência

Quando a lista muda, o fluxo é este, em linhas gerais: a escrita vai ao banco, depois um broadcast avisa o processo da fila, que atualiza a cópia em memória, e só então o evento sai para os clientes. Se o broadcast se perde, a cópia fica desatualizada até o processo reiniciar ou até a próxima mudança.

Como o CockroachDB é distribuído, a leitura logo depois da escrita pode, em teoria, pegar uma versão antiga se for feita fora da mesma transação. Até agora o fluxo lê dentro da transação, então não vi isso acontecer, mas é o primeiro lugar que eu olharia se alguém reclamar de moderador "fantasma".

Sintomas conhecidos de dessincronia:

- Moderador recém-adicionado vê a tela de moderação mas toma recusa no primeiro comando.
- Moderador removido continua vendo os botões até recarregar, mas os comandos falham. Isso é esperado.
- Depois de um failover de nó, a cópia em memória foi recarregada com a lista de antes de uma edição recente. Aconteceu uma vez em teste, não reproduzi de forma confiável.

## Operação durante o evento

Para quem opera, o que importa na prática:

- Adicionar moderador no meio do evento funciona e não exige reiniciar nada. Melhor fazer antes de o pico de perguntas começar.
- Evitar editar a lista inteira de uma vez durante o pico; adicionar ou remover um a um gera menos broadcasts e é mais fácil de acompanhar nos logs.
- Se um moderador reclamar que perdeu acesso, pedir para recarregar a página antes de qualquer outra coisa. Resolve a maior parte dos casos.
- Se recarregar não resolver, conferir se a pessoa ainda está na lista no banco e se o processo da fila daquele evento está vivo.

Os logs do qa-queue registram a mudança da lista com o evento e o usuário afetado. Não registram o conteúdo completo da lista, então para reconstruir o estado é preciso ler o banco.

## Pontos em aberto

Coisas que não verifiquei ou que ficaram pendentes:

- Não sei se o limite de moderadores é aplicado também na importação em lote ou só na interface. Vale testar.
- A auditoria de quem adicionou quem é parcial; há o registro da mudança, mas nem sempre o autor aparece quando a ação vem de uma integração.
- Falta teste automatizado para o caso de broadcast perdido. Hoje a recuperação depende do reinício do processo.
- Convém decidir se a presença deve passar a mostrar o papel do moderador ao lado do nome, o que ajudaria produtores a ver quem é quem sem abrir a lista.
- Revisar se a cópia em memória deveria ter uma releitura periódica do banco como rede de segurança. Custo baixo, mas ninguém decidiu ainda.

## Onde olhar primeiro

Quando algo estiver errado com moderadores no qa-queue, minha ordem é: papel do usuário, lista no banco, cópia em memória do processo da fila, e só por último o cliente. Quase todo problema que vi estava nos dois primeiros itens ou numa aba desatualizada.
