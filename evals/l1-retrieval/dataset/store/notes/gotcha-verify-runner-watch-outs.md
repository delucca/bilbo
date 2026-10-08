---
id: 01KSQ51V1ZMCJVEPF582RQ0QXG
created: 2026-05-28T08:20-03:00
---

# verify-runner: cuidados gerais ao mexer

Notas rápidas do que costuma dar problema quando alguém mexe no verify-runner. Não é uma lista de regras fechadas, é o que vale lembrar antes de abrir o editor. O verify-runner é a parte do PatchPilot que pega um pull request de upgrade de dependência e roda os testes direcionados para dizer se a atualização é segura. Se ele erra, o erro aparece em muitos repositórios ao mesmo tempo, e quem sofre é o engenheiro de plataforma que confiava no sinal verde.

## Onde o verify-runner fica no fluxo

Antes de mudar qualquer coisa, tenha o desenho na cabeça. O fluxo básico é este:

```
PR de upgrade -> verify-runner -> testes direcionados -> resultado em SQLite
```

O verify-runner recebe o contexto de um PR, decide o que rodar, executa em um ambiente isolado (Docker, na maior parte dos casos, ou direto no runner do GitHub Actions), coleta a saída e grava o resultado. Cada etapa tem suas falhas próprias. Quando algo quebra, a primeira pergunta é em qual etapa quebrou, e não se o teste falhou. Muita mudança mal feita vem de misturar essas etapas: um erro de infraestrutura sendo tratado como falha de teste, ou o contrário.

Outra coisa: o verify-runner é chamado por outras partes do PatchPilot. Mudar o formato do que ele devolve afeta quem lê o resultado, inclusive scripts de relatório que ninguém lembra que existem. Procure os consumidores antes de renomear campo ou mudar o significado de um status.

## Falha de infraestrutura não é falha de teste

Esse é o erro mais comum. O verify-runner pode terminar sem sucesso por motivos que não têm nada a ver com a dependência atualizada: imagem que não baixou, rede instável, disco cheio, timeout do job, runner do GitHub Actions que foi reciclado no meio. Se tudo isso for tratado como teste vermelho, o PR de upgrade fica marcado como quebrado sem estar, e alguém perde tempo investigando uma atualização inocente.

O contrário também machuca. Se o verify-runner engole uma falha real porque a saída veio num formato inesperado, o PR passa como verde sem ter sido verificado de verdade. Prefira sempre um estado explícito de "não consegui verificar" a um verde por omissão. Ao adicionar um novo caminho de erro, pergunte em que categoria ele cai e se o consumidor consegue distinguir.

## Seleção de testes direcionados

O ponto do verify-runner é rodar só o que importa para aquela dependência. Isso depende de um mapeamento entre o pacote atualizado e os testes que o usam. Esse mapeamento é aproximado, e quase toda mudança na seleção troca precisão por cobertura.

Cuidados:

- Se a seleção ficar estreita demais, um upgrade que quebra um uso indireto passa sem ser notado.
- Se ficar larga demais, o tempo de verificação explode e as pessoas começam a ignorar o resultado.
- Dependências transitivas e dependências de desenvolvimento têm comportamentos diferentes; não assuma que a regra de uma vale para a outra.
- Monorepos têm vários pacotes com suas próprias configurações de teste. A seleção precisa respeitar a fronteira de cada um.
- Quando o mapeamento não encontra nada, não devolva sucesso vazio. Zero testes rodados não significa zero problemas.

Sempre que alterar a lógica de seleção, teste com um repositório pequeno e outro grande, e olhe o que foi selecionado, não só se passou.

## Isolamento e Docker

O código de um PR de upgrade é código de terceiros: a nova versão de uma dependência pode rodar scripts na instalação. O verify-runner precisa tratar isso como não confiável. Ao mexer na parte de contêineres, tenha cuidado com o que é montado dentro do contêiner, com quais variáveis de ambiente entram, e com acesso à rede durante a execução dos testes.

Segredos do repositório ou do PatchPilot não devem vazar para dentro do ambiente de teste só porque é prático. Se um teste precisa de credencial, isso deve ser uma decisão explícita e visível, não um efeito colateral de herdar o ambiente inteiro.

Também vale lembrar da limpeza. Contêineres, volumes e diretórios temporários que sobram depois de uma execução acumulam rápido quando o volume de PRs é alto. Toda mudança no ciclo de vida do ambiente deve garantir a limpeza também nos caminhos de erro e de cancelamento, não só no caminho feliz.

## Concorrência e limites de recursos

O PatchPilot abre muitos PRs em muitos repositórios, e o verify-runner vai receber picos. Mudanças que parecem inocentes, como paralelizar mais uma etapa, podem esgotar memória, CPU ou os limites do runner do GitHub Actions. Antes de aumentar paralelismo, pense no pior caso e não na média.

Outros pontos:

- Duas verificações do mesmo repositório rodando ao mesmo tempo podem disputar cache, portas, diretórios de trabalho ou o mesmo banco local de testes.
- Reexecuções automáticas multiplicam a carga. Uma política de retry mal calibrada transforma uma instabilidade pequena numa avalanche.
- Timeouts precisam existir em todas as etapas que falam com algo externo. Um processo pendurado segura um slot para sempre.
- Cancelar uma verificação deve realmente matar os processos filhos, não só parar de esperar por eles.

## Estado no SQLite

O resultado das verificações fica em SQLite. É simples e funciona bem, mas tem armadilhas conhecidas quando há vários escritores. Escritas concorrentes podem travar ou falhar por banco ocupado se o verify-runner não tratar isso. Mantenha transações curtas e nunca segure uma transação aberta enquanto espera um teste terminar.

Mudanças de esquema merecem atenção especial. Há bancos já existentes em uso, e uma migração que assume banco vazio vai quebrar na primeira atualização real. Escreva a migração pensando em dados antigos, em linhas incompletas e em execuções que estavam no meio quando o processo reiniciou. Evite mudar o significado de uma coluna existente sem migrar os dados; é melhor adicionar uma nova.

Também não confie que o registro de uma execução está sempre completo. Se o processo morreu no meio, pode haver um registro de "em andamento" que nunca será fechado. O código que lê o estado precisa lidar com isso, e o que escreve deveria ter um jeito de reconciliar.

## Idempotência e reexecução

O mesmo PR pode ser verificado mais de uma vez: um novo commit, um rebase, uma reexecução manual, um evento duplicado do GitHub. O verify-runner deve se comportar bem nesses casos. Isso quer dizer que processar o mesmo evento duas vezes não deve gerar dois resultados conflitantes nem comentários duplicados no PR.

Pense também em qual resultado vale quando duas execuções se sobrepõem: a mais recente deve vencer, e uma execução antiga que termina tarde não pode sobrescrever o resultado de uma nova. Esse tipo de corrida só aparece em produção, então vale revisar com calma qualquer código que grave resultado ao final da execução.

## Integração com GitHub Actions

Parte da execução depende do GitHub Actions, e isso traz comportamentos que não controlamos: limites de uso da API, eventos que chegam fora de ordem ou atrasados, permissões do token que variam por repositório e PRs vindos de forks com permissões reduzidas.

Ao mudar algo aqui:

- Não assuma que o token tem permissão de escrita. Trate a ausência de permissão como um caso normal.
- Respeite os limites da API; chamadas em laço sobre muitos repositórios esgotam a cota rápido.
- Não dependa da ordem dos eventos.
- Se alterar o formato do status ou do comentário publicado no PR, lembre que pessoas e automações leem isso. Mudança de texto quebra filtros de quem automatizou em cima.

Se o workflow mudar, confira também os repositórios que usam uma versão antiga dele. Nem todo mundo atualiza junto.

## Saída, logs e dados sensíveis

Os testes escrevem muita coisa na saída, e o verify-runner guarda ou repassa parte disso. Duas preocupações. Primeira: segredos podem aparecer em log, seja por um teste que imprime a configuração, seja por uma ferramenta que ecoa variáveis. Qualquer caminho que publica log num PR ou o grava de forma durável deveria passar por mascaramento. Segunda: tamanho. Uma saída enorme pode estourar memória se for acumulada inteira em buffer. Prefira transmitir em fluxo e truncar de forma explícita, avisando que houve truncamento.

Ao truncar, preserve o fim da saída, porque é onde costuma estar a causa da falha. E não faça parsing frágil da saída humana das ferramentas de teste: um formato legível por máquina, quando existe, é mais estável que expressões regulares sobre texto.

## Testes do próprio verify-runner

É fácil testar o verify-runner só com mocks e ficar com falsa confiança. Os mocks confirmam que o código chama o que deveria, não que o comportamento real com Docker, processos e arquivos funciona. Mantenha pelo menos alguns testes que exercitam o fluxo de ponta a ponta contra um repositório de exemplo pequeno.

Alguns casos que merecem teste e são esquecidos com frequência:

- teste que trava e precisa de timeout
- processo que ignora o pedido de parada
- saída vazia
- repositório sem nenhum teste selecionado
- banco com dados de uma versão anterior
- cancelamento no meio da execução

Teste flaky no próprio verify-runner é especialmente perigoso, porque ele é o árbitro dos outros. Se um teste dele é instável, conserte ou remova; não deixe com retry automático escondendo o problema.

## Antes de publicar uma mudança

Uma checagem curta, feita sem pressa mesmo quando a mudança parece pequena:

- Mudou o que é devolvido ou gravado? Procure quem consome.
- Mudou o esquema do SQLite? Rode a migração sobre um banco antigo real.
- Criou um novo caminho de erro? Defina se é falha de infraestrutura ou de teste.
- Mexeu em isolamento? Releia o que entra no contêiner.
- Mexeu em concorrência? Pense no pico, não na média.
- Há limpeza em todos os caminhos de saída?

Se algo disso ficar incerto, publique de forma gradual, em poucos repositórios primeiro, e observe antes de abrir para todos. Um erro do verify-runner se espalha pela frota inteira muito mais rápido do que se corrige.

## O que ainda precisa de atenção

Algumas áreas do verify-runner são mais frágeis do que deveriam e merecem cuidado extra sempre que forem tocadas: a ligação entre seleção de testes e dependências indiretas, a reconciliação de execuções interrompidas e o tratamento de saída muito grande. Se você resolver uma delas de passagem, registre o que descobriu neste mesmo assunto em vez de abrir uma nota nova, para a próxima pessoa não refazer a investigação.
