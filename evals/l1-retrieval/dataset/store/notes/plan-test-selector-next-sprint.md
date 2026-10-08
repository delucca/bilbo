---
id: 01K99Y93JNZEN2SQGFAAG40MW3
created: 2025-11-05T09:01-03:00
---

# Plano: cache do grafo de imports no test-selector

Plano para a próxima sprint: o `test-selector` vai guardar em cache o grafo de imports, com a chave vinda da saída de `git rev-parse HEAD^{tree}`. Quando a árvore não mudou desde uma execução anterior, o `test-selector` pula a construção do grafo e usa o que já está salvo. Este documento lista o que fazer, em que ordem e o que fica de fora. Anotado às pressas, então tem coisa em aberto, e está marcada como tal.

## Escopo da sprint

Entra: leitura da chave de cache a partir de `git rev-parse HEAD^{tree}`, gravação do grafo serializado depois da construção, leitura do grafo salvo no começo de uma execução, e um caminho de fallback para construir o grafo do zero quando não há entrada válida. Entra também uma forma de ver nos logs se a execução usou o cache ou construiu o grafo.

Não entra: cache de resultados de teste, cache entre repositórios diferentes, compartilhamento do cache entre runners do GitHub Actions por rede, e qualquer mudança no algoritmo que escolhe os testes a partir do grafo. O `test-selector` continua escolhendo testes do mesmo jeito; só muda de onde o grafo vem.

## Comportamento esperado

No começo de uma execução, o `test-selector` roda `git rev-parse HEAD^{tree}` dentro do checkout do repositório alvo e usa a saída como chave. Se existe entrada para essa chave, carrega o grafo e segue direto para a seleção. Se não existe, constrói o grafo como hoje, grava a entrada e segue.

Duas execuções sobre árvores idênticas, mesmo com commits diferentes, caem na mesma chave. Isso é o comportamento desejado e precisa de teste: um commit que só altera mensagem ou autoria não deve gerar nova construção do grafo.

Uma execução sobre árvore diferente nunca lê a entrada de outra árvore. Não há reaproveitamento parcial nesta sprint: ou a árvore inteira bate, ou o grafo é reconstruído por completo.

## Armazenamento

O grafo fica em SQLite, no mesmo banco local que o PatchPilot já usa para estado de execução, em uma tabela nova só para o cache. Cada linha guarda a chave da árvore, o grafo serializado, a data de criação e a versão do formato de serialização. A chave da árvore é única na tabela.

A versão do formato entra na leitura: se a versão gravada difere da versão que o código atual espera, a entrada é tratada como ausente e o grafo é reconstruído. A entrada antiga é sobrescrita pela nova.

Detalhe aberto: definir se o grafo serializado vai como um único blob ou dividido por módulo. Começar pelo blob único; dividir só se a leitura ficar lenta na prática.

## Serialização do grafo

O grafo hoje existe só em memória, como mapa de arquivo para lista de arquivos que ele importa. Para o cache, escrever funções de serialização e desserialização em TypeScript, com teste de ida e volta: serializar, desserializar e comparar com o original.

A ordem dos nós e das arestas precisa ser estável na serialização, para que o mesmo grafo gere sempre os mesmos bytes. Isso facilita comparar entradas em depuração e testar.

Não serializar caminhos absolutos do runner. Gravar caminhos relativos à raiz do repositório e remontar na leitura.

## Passos de implementação

Primeiro, isolar a construção do grafo atrás de uma interface pequena no `test-selector`, com uma operação que devolve o grafo para uma árvore. A implementação atual vira a implementação sem cache.

Segundo, escrever o módulo que obtém a chave chamando o Git e tratando falhas: repositório sem commit, diretório que não é repositório, saída vazia. Em qualquer falha, o `test-selector` segue sem cache e registra um aviso no log.

Terceiro, escrever a camada de armazenamento no SQLite, com criação da tabela na primeira execução e migração simples se a tabela já existir com formato antigo.

Quarto, escrever a implementação com cache que envolve a implementação sem cache: consulta, devolve se achou, senão constrói, grava e devolve.

Quinto, ligar tudo na configuração do `test-selector` com uma opção para desligar o cache por execução.

Sexto, testes e documentação interna.

## Testes

Testes unitários: serialização de ida e volta, leitura de entrada com versão de formato diferente, leitura de entrada ausente, gravação sobre entrada existente.

Testes de integração com um repositório Git temporário: duas execuções seguidas na mesma árvore devem construir o grafo uma vez só; uma alteração em arquivo versionado deve gerar construção nova; um commit vazio ou só de mensagem deve reaproveitar a entrada.

Teste de falha: banco de cache ilegível ou corrompido. O `test-selector` deve ignorar o cache, construir o grafo e terminar a execução normalmente, com aviso no log.

Rodar a suíte no GitHub Actions e também dentro do contêiner Docker usado nas verificações, para pegar diferença de comportamento do Git entre os dois ambientes.

## Integração com GitHub Actions

O banco de cache precisa sobreviver entre execuções de workflow para ser útil. Duas opções: usar o mecanismo de cache de artefatos do próprio GitHub Actions para salvar e restaurar o arquivo do banco, ou montar um volume persistente quando o runner é auto-hospedado. Decidir qual entra na sprint; a primeira é a candidata principal por não exigir infraestrutura extra.

Seja qual for a opção, o workflow restaura o arquivo antes de chamar o `test-selector` e salva depois. O `test-selector` em si não sabe nada sobre o mecanismo; ele só abre o arquivo no local configurado.

Se a restauração falhar ou o arquivo não existir, é o mesmo caso de entrada ausente: constrói o grafo e segue.

## Imagem Docker

A imagem usada para rodar o `test-selector` precisa ter o Git instalado e um diretório gravável para o banco de cache. Verificar a imagem atual e ajustar o Dockerfile se faltar algum dos dois. A publicação da imagem em si está descrita em [[sandbox-image-published-registry]]; este plano não altera esse processo.

O checkout dentro do contêiner pode ter dono diferente do usuário que roda o processo, e o Git pode recusar o diretório. Testar o comando de chave dentro da imagem antes de considerar o passo pronto, e configurar o diretório como seguro se for preciso.

## Observabilidade

Cada execução do `test-selector` registra uma linha de log dizendo se o grafo veio do cache ou foi construído, a chave usada e o tempo gasto nessa etapa. Em caso de entrada descartada por versão de formato diferente, registrar isso explicitamente.

Acrescentar ao resumo final da execução um campo indicando acerto ou falha de cache, para aparecer no comentário que o PatchPilot deixa no pull request de atualização de dependência.

Medir o tempo de construção do grafo antes e depois em alguns repositórios de tamanhos diferentes e anotar os resultados neste documento ao final da sprint.

## Riscos e pontos em aberto

Árvore de trabalho suja: se o checkout tem mudanças não commitadas, a saída do comando de chave não reflete o conteúdo real. No fluxo normal do PatchPilot o checkout é limpo, mas o `test-selector` deve checar isso e, estando sujo, ignorar o cache.

Submódulos: o grafo pode incluir arquivos de submódulos, e a chave da árvore do repositório principal não cobre o conteúdo deles do mesmo jeito. Decidir se o `test-selector` desliga o cache quando há submódulos ou se inclui o estado deles na chave. Em aberto.

Crescimento do banco: sem política de remoção, a tabela só aumenta. Colocar uma limpeza simples das entradas mais antigas na primeira versão, com limite configurável, ou deixar para a sprint seguinte. Em aberto.

Concorrência: duas execuções simultâneas gravando a mesma chave. A gravação deve ser tolerante a isso, mantendo uma só entrada.

## Critérios de pronto

O `test-selector` usa `git rev-parse HEAD^{tree}` como chave e pula a construção do grafo quando há entrada válida. Execuções repetidas sobre a mesma árvore mostram acerto de cache nos logs. Falhas de Git, de banco ou de formato nunca derrubam a execução; caem no caminho sem cache. Os testes unitários e de integração passam no GitHub Actions e no contêiner Docker. A opção de desligar o cache funciona. Os números de antes e depois estão anotados aqui, e os pontos em aberto acima têm decisão registrada ou foram adiados de forma explícita.
