---
id: 01KZ4ND8XY6ABE71VG8NNQ44KZ
created: 2026-08-03T17:35-03:00
---

# analyst-orders-api: respostas http que parecem certas e não estão

Anotação rápida, feita com pressa, sobre o jeito como o `analyst-orders-api` responde por http quando o analista pede ordens de reposição. Não é a nota completa, é o que lembro e o que anotei durante a investigação. Os valores exatos (limites, tempos, códigos) ficam de fora de propósito: confira na configuração do serviço antes de confiar em qualquer coisa daqui.

A ideia central: a resposta http do `analyst-orders-api` nem sempre quer dizer o que parece. Uma resposta de sucesso pode trazer ordens velhas, incompletas ou geradas para um recorte diferente do que o analista pediu. E uma resposta de erro às vezes esconde que o trabalho já foi feito do lado de dentro. Quem consome a API sem olhar o corpo e os campos de controle acaba tomando decisão de reposição em cima de dado que não corresponde à loja ou ao dia esperado.

Quem mais sofre com isso é o analista de merchandising, que abre a tela, vê uma lista bonita e assume que é a previsão mais recente de ruptura. Em redes de supermercado isso custa caro: pedido errado de perecível, prateleira vazia, ou excesso que estraga. Por isso vale deixar escrito onde está a armadilha, mesmo que a explicação ainda esteja parcial.

## Sintoma principal

O sintoma que mais apareceu: a chamada volta com sucesso, o corpo vem preenchido, mas as ordens não batem com o que o pipeline gerou na última rodada. O analista reclama que "a ordem sumiu" ou "a quantidade mudou sozinha". Quando a gente compara com a tabela de origem, os dados novos existem lá, só não chegaram na resposta.

Outro sintoma, menos frequente: a resposta demora até estourar o limite configurado no gateway e o cliente recebe uma falha de tempo esgotado. Só que o serviço continua trabalhando e, minutos depois, a mesma consulta já volta rápida e completa. Parece que a primeira chamada aqueceu algo. Isso confunde, porque o analista tenta de novo e "funciona", e ninguém abre chamado.

Um terceiro: respostas parciais. O corpo vem com menos lojas do que o filtro pedia, sem nenhum aviso explícito. Não há erro, não há campo dizendo que cortou. O analista só percebe quando conta as lojas na mão ou quando uma loja específica nunca aparece.

## Por que o código http engana

O ponto mais traiçoeiro é que o serviço usa o código de sucesso para casos em que, a rigor, deveria avisar algo diferente. Quando a leitura da camada de dados volta vazia ou incompleta, o handler monta uma resposta normal com lista reduzida em vez de sinalizar problema. Do ponto de vista do cliente http, está tudo certo.

Também há o caminho contrário: quando a consulta estoura o tempo, o gateway devolve falha, mas o serviço não cancela o que estava rodando. Então a falha http não significa que nada aconteceu. Em um caso a ordem foi gravada mesmo assim, e o retry do analista gerou uma segunda tentativa em cima da primeira. Não confirmei se há idempotência real nesse fluxo; tratem como não garantida até alguém verificar no código.

Resumindo o que não se pode assumir:

- sucesso na resposta não prova que os dados são os mais recentes;
- sucesso na resposta não prova que todas as lojas pedidas vieram;
- falha na resposta não prova que a operação não foi executada;
- repetir a chamada não é necessariamente seguro quando ela cria ou confirma ordens.

## Quando costuma acontecer

Aparece mais perto do horário em que o Airflow termina a rodada diária de previsão e geração de ordens, e logo depois, quando os analistas entram para revisar. É a janela em que a tabela está sendo reescrita e o serviço lê um estado intermediário. Fora dessa janela o comportamento é bem mais estável.

Também piora quando muitos analistas consultam ao mesmo tempo, especialmente os que pedem várias lojas de uma vez, ou uma rede inteira. Nesses casos o tempo de resposta sobe e a chance de cair no limite do gateway aumenta. Não tenho a medida exata; é impressão de quem acompanhou os logs, e o valor usual do limite está na configuração do gateway.

Uma terceira situação: depois de reprocessamento manual de um dia anterior. Se alguém reroda a previsão de um período passado, a tabela de ordens muda retroativamente, e a API, dependendo do cache, devolve ora a versão antiga, ora a nova, na mesma tela.

## Hipóteses sobre a causa

Ainda não está fechada. As hipóteses que sobraram, da mais provável para a menos:

1. Cache na frente da consulta, com validade maior do que o intervalo entre rodadas importantes. O analista vê o resultado em cache enquanto a tabela já foi atualizada.
2. Leitura sem garantia de versão consistente: o serviço lê a tabela enquanto o job escreve, e pega um conjunto parcial. Com Delta Lake isso deveria ser raro por causa das transações, a menos que a leitura esteja fixada em uma versão antiga ou use uma visão materializada defasada.
3. O handler engole exceções da camada de dados e responde como sucesso com lista menor. Precisa ler o código para confirmar onde o erro é capturado.
4. Diferença de fuso ou de data de corte entre o que o analista pede e o que o job considera "o dia de hoje". Isso explicaria a ordem que "some" perto da virada do dia.

Nenhuma dessas foi descartada. Provavelmente mais de uma está acontecendo ao mesmo tempo, o que explica por que o sintoma muda de caso para caso.

## Camada Spark e Delta Lake

O job em Scala no Spark gera as ordens e grava em tabelas Delta. A API não roda Spark por requisição; ela lê o resultado já materializado. Então qualquer defasagem entre o fim do job e o que a API enxerga vem da leitura, não do cálculo.

Coisas a verificar quando o analista diz que a ordem está errada:

- se a escrita do job terminou de fato antes da consulta (o job pode ter terminado com sucesso parcial);
- se a leitura da API usa a versão mais recente da tabela ou uma versão fixada;
- se houve compactação ou limpeza de arquivos que deixou uma leitura longa sem os arquivos que ela esperava;
- se o particionamento usado no filtro da API é o mesmo que o job usou para escrever, porque um filtro que não aproveita a partição deixa a consulta lenta e aumenta a chance de estourar o tempo.

Um detalhe que me custou tempo: olhar a tabela pelo notebook mostra o estado atual, enquanto a API pode estar mostrando outro. Comparar os dois sem alinhar a versão leva a conclusão errada, tipo "a API está bugada" quando na verdade ela leu um instante antes.

## Snowflake e a segunda fonte

Parte do que o analista vê vem do Snowflake, não do Delta. Dados de cadastro de loja, calendário de promoção e alguns agregados históricos moram lá. A API junta as duas fontes na hora de montar a resposta, e a junção é onde mais aparece o corte silencioso de lojas.

Se uma loja existe nas ordens mas não casa com o cadastro, ela some da resposta em vez de vir com um campo faltando. Isso bate com o relato de loja que nunca aparece. Vale conferir chaves de loja que mudaram de formato, lojas novas ainda sem cadastro completo e lojas fechadas ou em reforma que foram marcadas como inativas.

Também há a questão de quando o Snowflake foi atualizado. Se a carga dele roda em horário diferente da do Airflow para o Delta, existe uma janela em que as duas fontes discordam, e a junção produz resposta com mistura de dias. Não achei documentação clara sobre a ordem esperada entre as duas cargas.

## Airflow e o horário das rodadas

O DAG que dispara a previsão e a geração de ordens é o marco para tudo isso. A API não sabe se a rodada do dia terminou; ela só lê o que há. Seria bom que o fim da rodada publicasse um sinal (um marcador de "dia concluído") que a API consultasse, e que respondesse de forma explícita quando a rodada ainda estivesse em andamento.

Hoje, quando uma tarefa do DAG falha ou atrasa, a API continua servindo a rodada anterior sem nenhuma indicação. O analista não tem como saber. Já vi o caso de uma tarefa reexecutada manualmente depois do horário em que os analistas começam a trabalhar, e a tela mudou no meio da revisão.

Sugestão que ficou na conversa e não foi implementada: incluir no corpo da resposta um campo com a identificação da rodada e o momento em que ela terminou, e fazer o cliente destacar quando isso for mais antigo do que o esperado para aquele dia.

## O que o analista enxerga

Do lado de quem usa, a queixa é sempre em linguagem de negócio: "a ordem da loja tal não está certa", "falta produto na lista", "ontem estava diferente". Raramente vem com horário da consulta ou filtros usados, o que atrasa a investigação. Peça sempre: quais lojas, qual dia de referência, que horas abriu a tela e se recarregou.

Outra coisa: a tela cacheia no navegador e no próprio front. Parte das reclamações de dado velho era só isso. Uma recarga forçada resolveu algumas. Não dá para descartar o cache do serviço só porque o do navegador foi limpo; são camadas separadas e a gente já confundiu as duas.

Também é bom lembrar que o analista toma decisão com a lista. Se uma ordem está ausente por falha de leitura, ele pode interpretar como "a previsão diz que não há risco de ruptura", que é o pior tipo de erro: a ausência parece informação.

## Como contornar por enquanto

Enquanto a causa não for resolvida, o que tem funcionado, sem garantia:

- conferir sempre a identificação da rodada no corpo da resposta, quando existir, antes de aceitar a lista;
- comparar a quantidade de lojas devolvidas com a quantidade pedida; qualquer diferença é suspeita;
- evitar consultas muito amplas na janela logo após o fim do DAG; pedir por grupos menores de lojas;
- não repetir automaticamente chamadas que criam ou confirmam ordens depois de falha de tempo; primeiro consultar o estado atual;
- em caso de dúvida, olhar a tabela de origem direto, alinhando a versão, e só depois acusar a API ou o job.

Para quem for mexer no serviço: faça a resposta distinguir "vazio de verdade" de "não consegui ler", e faça o corte de lojas aparecer num campo de avisos. Isso sozinho eliminaria boa parte do problema, mesmo sem mexer em cache.

## O que ainda não está claro

Lista do que ficou em aberto, para quem pegar isso depois:

- onde exatamente o handler captura erro da camada de dados e transforma em lista menor;
- qual é a validade real do cache e se ela é a mesma para todos os tipos de consulta;
- se há mesmo idempotência na confirmação de ordens ou se o retry duplica;
- a ordem esperada entre a carga do Snowflake e a escrita no Delta;
- se o corte silencioso por falta de cadastro é intencional ou sobra de uma junção interna que deveria ser externa;
- se o limite do gateway e o tempo máximo interno do serviço estão alinhados, ou se o gateway corta antes de o serviço desistir.

Não afirmo nada disso como fato. Foram coisas que apareceram na leitura dos logs e em conversas e que precisam de confirmação no código.

## Verificação rápida

Um esboço do caminho de dados, só para lembrar onde olhar em cada passo quando a resposta parecer estranha:

```text
Airflow -> Spark (Scala) -> Delta Lake -> analyst-orders-api -> analista
                                Snowflake ---^
```

Ordem de checagem que usei: primeiro o fim da rodada no Airflow, depois a versão da tabela no Delta, depois o cadastro no Snowflake, depois o cache do `analyst-orders-api`, e só por último o cliente do analista. Na maioria das vezes o problema estava entre o segundo e o quarto passo. Se alguém achar a causa de verdade, atualizem a nota principal sobre as respostas http do `analyst-orders-api` em vez de abrir outra: já existe mais de uma sobre o mesmo assunto e isso está ficando confuso.
