---
id: 01KWVCPZ7EM2STY1R4V1ZTD6R2
created: 2026-07-06T06:38-03:00
---

# Referência geral do carrier-sync-job

Nota de referência rápida sobre o carrier-sync-job: onde ficam as peças e como elas se ligam, em termos gerais. Não tem valores exatos de propósito. Para qualquer detalhe fino, abrir o código e a configuração do ambiente, não confiar só nesta nota.

O carrier-sync-job existe para manter o FreightWeave alinhado com o que as transportadoras dizem sobre seus veículos, trens e janelas de operação. O planejador de rotas multimodais (caminhão e trem) e o rebalanceamento de cargas dependem desses dados estarem razoavelmente frescos. Quando o dado fica velho, o plano sai errado e o despachante regional percebe primeiro.

## Papel do componente

O carrier-sync-job puxa informação de fontes das transportadoras, normaliza para o modelo interno e entrega o resultado para o resto do sistema. Ele não planeja rota e não decide rebalanceamento. Quem faz isso é o código de otimização, que usa OR-Tools. O job só garante que a entrada desse código reflita a realidade o melhor possível.

Pensar nele como uma ponte: lado de fora, os sistemas de cada transportadora, cada um com seu formato e seus defeitos; lado de dentro, o modelo único do FreightWeave.

## Onde o código mora

O código do job fica no mesmo repositório Python do restante do backend, em um pacote próprio ou em um subpacote dedicado a integrações com transportadoras. Procurar por pastas com nome ligado a carrier, sync ou integrations. Se a estrutura mudou, a busca pelo nome do componente nos arquivos de configuração costuma achar o ponto de entrada.

A separação habitual é esta: um módulo de entrada que orquestra a execução, módulos de adaptadores por transportadora, módulos de normalização e um módulo de publicação dos resultados. Os nomes exatos variam, então conferir no código.

## Ponto de entrada

O job é executado como processo de lote, não como endpoint de usuário. O ponto de entrada monta a configuração, abre as conexões necessárias e percorre as transportadoras habilitadas. Ele pode ser disparado por agendador externo ou por mensagem; confirmar no deploy qual dos dois está em uso hoje.

Se o disparo for por mensagem, o consumidor fica perto do ponto de entrada e o corpo da mensagem diz o que sincronizar. Se for por agendamento, a definição do agendamento mora na infraestrutura, não no código Python.

## Adaptadores por transportadora

Cada transportadora tem um adaptador que sabe falar com a fonte dela. Alguns usam API, outros arquivos trocados periodicamente, outros formatos antigos. O adaptador cuida de autenticação, paginação, repetição de tentativas e tradução dos campos crus.

Regra prática: quirks de uma transportadora ficam dentro do adaptador dela. Se uma exceção específica vazou para o código comum, provavelmente está no lugar errado e vale mover.

## Normalização

Depois do adaptador, os registros passam pela normalização. Aqui se unificam unidades, fusos horários, códigos de local e tipos de modal. Os modelos de dados costumam ser classes tipadas, as mesmas que a API FastAPI usa para validar entrada e saída, então mudanças de campo afetam os dois lados.

Registros que não fecham com o modelo devem ser descartados com registro em log ou separados para revisão, e não corrigidos em silêncio. Isso importa porque dado errado de capacidade leva o otimizador a planos inviáveis.

## Configuração

A lista de transportadoras habilitadas, as credenciais e os intervalos de sincronização vêm de configuração por ambiente. Segredos ficam no gerenciador de segredos da nuvem, nunca no repositório. Parâmetros não sensíveis ficam em arquivos de configuração ou variáveis de ambiente lidas na subida.

Ao investigar comportamento estranho, comparar a configuração do ambiente com a de outro ambiente que funciona. Muita falha vem de diferença aí e não de código.

## Uso do Redis

O Redis serve como cache e como armazenamento de estado leve do job. Em geral guarda a marca de progresso por transportadora, travas para evitar duas execuções concorrentes sobre a mesma fonte e cópias recentes dos dados normalizados que o planejador lê com rapidez.

As chaves seguem um prefixo por finalidade. Antes de limpar qualquer coisa no Redis, olhar o prefixo e entender o que depende dele. Apagar a marca de progresso força uma sincronização completa, que pode ser pesada para a fonte externa.

## Pub/Sub

O Google Cloud Pub/Sub é o canal para avisar o restante do sistema que há dado novo ou mudança relevante, como atraso de um trem ou indisponibilidade de um veículo. O job publica em tópicos; quem precisa reagir assina. O rebalanceamento de cargas é o consumidor mais importante desses eventos.

Mensagens devem ser tratadas como entregues pelo menos uma vez. Consumidores precisam tolerar duplicatas, e o publicador deve incluir informação suficiente para ordenar ou descartar eventos velhos.

## Relação com o planejador e o OR-Tools

O código de otimização não chama as transportadoras diretamente. Ele lê o que o job deixou no armazenamento e reage aos eventos. Por isso o formato do dado normalizado é um contrato: mudar significado de campo sem avisar quebra restrições do modelo de otimização sem erro visível.

Quando um plano parece absurdo, conferir primeiro se o dado de entrada está atual e coerente, e só depois suspeitar do modelo.

## Relação com a API FastAPI

A API expõe consultas e ações para os despachantes. Ela pode mostrar o estado da última sincronização por transportadora e, em alguns casos, pedir uma sincronização sob demanda. Esse pedido deve virar mensagem ou marca para o job, não execução dentro do processo da API.

Se a tela mostra dado velho, olhar a indicação de frescor que a API devolve antes de mexer no job.

## Tratamento de falhas

Falhas de fonte externa são normais: indisponibilidade, resposta incompleta, mudança de formato sem aviso. O job deve isolar cada transportadora para que uma falha não pare as demais. Há repetição com espera crescente para erros transitórios, e erro persistente deve gerar alerta, não loop infinito.

Quando uma transportadora fica fora por tempo longo, o sistema precisa deixar claro que o dado dela está velho, em vez de fingir que está atual.

## Idempotência

Rodar duas vezes a mesma sincronização não pode duplicar nem corromper dado. O desenho usa chaves naturais dos registros e gravação por substituição ou mesclagem. Ao mexer na gravação, manter essa propriedade, porque reentregas de mensagem e reexecuções manuais acontecem.

## Observabilidade

Logs estruturados por transportadora e por execução ajudam a seguir um problema. Métricas úteis: duração da execução, quantidade de registros aceitos e rejeitados, idade do dado mais recente e falhas por fonte. Os painéis e alertas ficam na ferramenta de monitoramento da nuvem; o código só emite os sinais.

Ao depurar, partir da idade do dado por transportadora. É o sintoma mais próximo do que o despachante sente.

## Testes

Os testes do componente ficam junto com o restante da suíte Python, normalmente espelhando a estrutura do pacote. Adaptadores são testados com respostas gravadas ou simuladas, sem chamar a fonte real. A normalização tem testes por tabela com casos de borda de unidade, fuso e código de local.

Para integração com Redis e Pub/Sub, usar emuladores ou instâncias descartáveis. Não apontar teste para recursos compartilhados.

## Execução local

Para rodar localmente, subir um Redis de desenvolvimento e um emulador do Pub/Sub, configurar as variáveis de ambiente do job com credenciais de teste e usar adaptadores simulados. Ver o README do repositório e a documentação interna de desenvolvimento para o passo a passo atual; não copiei comandos aqui para não envelhecer.

## Implantação

O job é empacotado em contêiner e roda na infraestrutura de nuvem do projeto. A definição de implantação, permissões de serviço e agendamento ficam no repositório de infraestrutura ou em arquivos de deploy dedicados. Mudanças de permissão do Pub/Sub e do acesso ao Redis passam por lá, não pelo código Python.

## Armadilhas comuns

Fusos horários: as fontes misturam horário local e universal, e o erro aparece como atraso fantasma. Códigos de local: transportadoras diferentes chamam o mesmo terminal de formas diferentes. Cache: dado velho no Redis pode mascarar uma correção recém-implantada. Eventos duplicados ou fora de ordem: o rebalanceamento deve ser robusto a isso.

Outra: ampliar o modelo normalizado sem versionar o contrato com os consumidores.

## Onde procurar primeiro

Dado velho ou ausente: estado da execução e marca de progresso no Redis, depois o log do adaptador da transportadora. Plano estranho: dado normalizado de entrada. Evento que não chegou: publicação no tópico e assinatura do consumidor. Falha só em um ambiente: diferenças de configuração e permissões.

## Lacunas desta nota

Esta nota é só um mapa. Não registra valores, nomes exatos de módulos, intervalos nem decisões de projeto. Se algo aqui divergir do código, o código vale, e esta nota deve ser corrigida.
