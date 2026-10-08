---
id: 01M2H6TXR8DAYRMYW2Q84PZR88
created: 2026-09-14T21:17-03:00
---

# Ledgerlark infra: anotações soltas sobre o plano do Terraform

Anotação rápida, feita sem reler nada do que já existia sobre o plano do Terraform do `ledgerlark-infra`. Pode repetir coisa, pode contradizer em algum ponto. Se contradisser, vale o que estiver no código e no estado real, não aqui. Não coloquei valores exatos de propósito: onde faltou certeza eu escrevi "o valor de sempre" ou "o limite configurado" e deixei assim.

O que o `ledgerlark-infra` cobre, em linhas gerais: a infraestrutura que sustenta o Ledgerlark, que é o serviço que concilia arquivos de liquidação das processadoras de cartão com os lançamentos do razão interno e marca as divergências para revisão. Quem usa são times de operações financeiras de marketplaces. Então o que importa na infra é que ela seja previsível, que o banco não perca dado e que a fila de eventos não engasgue na hora do fechamento.

```
ledgerlark-infra
  PostgreSQL
  Apache Kafka
  gRPC
  Terraform
```

O bloco acima é só o mapa mental que eu uso: o Terraform descreve o resto, e o resto é banco, mensageria e a camada de serviços falando gRPC entre si. Os serviços em Go ficam fora do escopo desta nota, exceto onde a infra os afeta.

## Como o plano é usado no dia a dia

A regra de ouro que eu tento seguir: ninguém aplica nada sem ter lido o plano inteiro antes. Parece óbvio, mas na prática o plano do `ledgerlark-infra` costuma ser longo, porque muitos recursos têm atributos computados que mudam de aparência sem mudar de verdade. O resultado é muito ruído, e o risco é a pessoa passar o olho, ver "tudo normal" e deixar passar uma destruição no meio.

O que eu faço é ler o plano em duas passadas. Na primeira, só procuro por destruições e substituições. Qualquer recurso marcado para ser destruído e recriado merece parar e pensar, principalmente se for algo com estado: instância de banco, volume, tópico com dados retidos. Na segunda passada leio as mudanças in-place, que quase sempre são tags, descrições, ajustes de parâmetros.

Quando o plano mostra substituição de algo com estado, a pergunta é sempre a mesma: por que o provedor acha que precisa substituir? Muitas vezes é um atributo que não pode ser alterado depois da criação, e alguém mudou o valor no código sem perceber. Nesse caso a correção é desfazer a mudança no código ou planejar uma migração de verdade, com janela e com backup. Não é aplicar e torcer.

Outra coisa que aprendi do jeito difícil: o plano é uma fotografia do momento em que foi gerado. Se ele fica parado esperando aprovação por muito tempo, o estado real pode ter mudado por baixo. Por isso, se passou tempo demais entre gerar e aplicar, eu gero de novo. Salvar o arquivo de plano e aplicar exatamente aquele arquivo ajuda a garantir que o que foi revisado é o que roda, mas só vale enquanto o estado não andou.

### Ambientes

Existem ambientes separados, cada um com seu estado remoto. A divisão em si não é novidade e não vou detalhar nomes aqui. O ponto é que os módulos são os mesmos e o que muda são as variáveis de cada ambiente. Isso é bom porque reduz a chance de o ambiente de teste ser diferente do real em estrutura, mas é ruim porque uma mudança de módulo afeta todos de uma vez quando cada um for aplicado.

A ordem que eu sigo: aplicar primeiro no ambiente menos crítico, olhar o resultado, e só depois subir para o mais crítico. O plano do ambiente crítico deve ser parecido com o do menos crítico, e se não for parecido eu quero entender a diferença antes de prosseguir.

As variáveis específicas de cada ambiente vivem em arquivos separados. Eu evito passar variável na linha de comando, porque isso some do histórico e depois ninguém sabe com que valor o plano foi gerado. Se precisar de uma exceção, escrevo na descrição da mudança.

### Estado remoto e travas

O estado fica em backend remoto, com trava para evitar duas execuções ao mesmo tempo. Quando uma execução morre no meio, a trava pode ficar presa. A tentação é soltar a trava à força na hora, mas antes eu confirmo que não há outra execução realmente em andamento. Soltar trava com execução viva é o jeito mais rápido de corromper o estado.

Se o estado ficar corrompido ou dessincronizado, a recuperação passa pelas versões antigas do arquivo de estado, que o backend guarda. Nunca editei o estado à mão sem ter uma cópia do estado atual guardada antes. Mesmo assim, prefiro os comandos próprios de manipulação de estado (mover, remover, importar) em vez de editar o arquivo.

## Banco PostgreSQL

O banco é o recurso mais sensível do `ledgerlark-infra`. É onde ficam os lançamentos do razão e os resultados da conciliação. Perder ou corromper isso é um problema financeiro para o cliente, não só técnico.

O plano do banco costuma ter pouca coisa, e quando tem muita coisa é sinal de alerta. O que normalmente aparece: ajustes em grupos de parâmetros, mudanças em regras de rede, alterações de tamanho da instância. Mudanças de parâmetro às vezes exigem reinício, e o plano nem sempre deixa isso claro. Eu confiro na documentação do provedor se o parâmetro é dinâmico ou estático antes de aplicar.

Backups e retenção são configurados no código, e a regra é não reduzir a retenção sem alinhamento. Já vi plano propondo reduzir retenção por causa de uma variável com valor padrão errado num ambiente novo. A leitura atenta pegou, mas poderia ter passado.

Proteção contra exclusão fica ligada nos ambientes que importam. Isso significa que um plano que tente destruir o banco vai falhar na aplicação, o que é bom, mas o plano em si ainda mostra a destruição normalmente. Não dá para confiar só na trava como substituta da leitura.

### Mudanças de tamanho e armazenamento

Aumentar armazenamento costuma ser tranquilo. Reduzir não é possível na maioria dos casos, e o plano vai tentar substituir o recurso se alguém diminuir o valor. Se isso aparecer, a mudança no código está errada, ponto. Para tamanho de instância, a troca causa uma janela de indisponibilidade curta, e o fechamento financeiro dos clientes tem períodos de pico em que isso não pode acontecer. Então conferir o calendário de fechamento faz parte de aplicar mudança de instância.

Existe uma conversa pendente sobre réplicas de leitura para aliviar consultas pesadas de relatório. Não está decidido, e não quero que esta nota passe a impressão de que está. Se for adiante, vira módulo novo e plano novo.

### Migrações de esquema

Migração de esquema não é responsabilidade do Terraform aqui. O Terraform cria o banco, os usuários de infraestrutura e a rede; o esquema é aplicado pelo fluxo de migrações do serviço. Misturar as duas coisas já causou confusão antes, com gente procurando tabela no plano. Se alguém perguntar onde mudar uma coluna, a resposta não é no `ledgerlark-infra`.

Senhas e credenciais do banco não devem aparecer em texto no plano. Se aparecerem, é bug de configuração de atributo sensível e precisa ser corrigido na hora, além de rotacionar a credencial exposta, caso o plano tenha sido colado em algum lugar público.

## Kafka e os tópicos

A mensageria com Apache Kafka carrega os eventos de liquidação e os eventos de lançamento que o conciliador consome. O plano do Terraform cobre tópicos, partições, retenção e as permissões de acesso por serviço.

A parte que mais dá dor de cabeça é a contagem de partições. Aumentar é possível, mas muda o particionamento por chave, e consumidores que dependem de ordem por chave podem ver ordem quebrada durante a transição. Reduzir não é suportado, então o provedor tende a propor recriação do tópico, o que apaga dados retidos. Essa é exatamente a situação de "substituição de recurso com estado" que descrevi acima, e deve parar qualquer aplicação até alguém confirmar.

A retenção dos tópicos tem relação direta com a capacidade de reprocessar. Se um arquivo de liquidação chega com problema e é preciso reprocessar, a retenção precisa ser suficiente para cobrir a janela. Por isso a retenção configurada nos tópicos de liquidação é maior do que nos tópicos de eventos mais efêmeros. Não vou escrever números; olhem as variáveis do módulo.

### Permissões e ACLs

Cada serviço tem seu usuário e suas permissões de leitura e escrita nos tópicos que usa. O plano mostra essas ACLs como recursos separados, e isso gera listas grandes. Quando um serviço novo entra, aparecem várias linhas de criação de uma vez. Eu confiro se a permissão pedida é a mínima necessária: serviço que só consome não recebe escrita.

Grupos de consumidores também têm permissão. Esqueci disso uma vez, e o serviço novo subiu mas não conseguia entrar no grupo. O sintoma no log do serviço não era claro, e demorou para ligar com a ACL. Fica o aviso: se um consumidor novo sobe e não recebe nada, olhar primeiro permissões de grupo.

### Compatibilidade com os consumidores

Antes de mexer em configuração de tópico, vale checar se algum consumidor depende do comportamento atual. Compactação de log, por exemplo, muda a semântica do que fica disponível para quem lê do começo. Os consumidores do conciliador, escritos em Go, assumem certas coisas sobre entrega, e não quero que uma mudança de infra quebre essa suposição silenciosamente.

## Rede, gRPC e segurança

A comunicação entre serviços usa gRPC, e isso tem consequência na infra: balanceamento de carga precisa entender conexões de longa duração. Um balanceador pensado para requisições curtas pode concentrar tráfego num único backend, porque a conexão é reaproveitada. A configuração de rede no `ledgerlark-infra` leva isso em conta, mas o plano não mostra a intenção, só os recursos. Quem for mexer precisa conhecer o motivo.

Regras de segurança de rede são o tipo de mudança que o plano mostra de forma enganosa. Uma regra substituída por outra equivalente aparece como remoção mais criação, e no intervalo entre as duas pode haver queda de conectividade. Quando o provedor permite criar antes de destruir, isso é mitigado, mas não é garantido em todos os recursos. Leio com cuidado qualquer plano que toque regras de entrada do banco ou do Kafka.

Certificados e termos de TLS entre serviços têm renovação própria. Se o plano propuser mexer em certificado perto da data de expiração, não é coincidência, e o melhor é tratar como manutenção planejada, não como ruído.

### Segredos

Segredos não ficam no repositório. O código referencia um gerenciador de segredos, e o plano só mostra referências. Se alguém precisar de um segredo novo, o fluxo é criar o segredo fora do código e referenciar. Já houve discussão sobre como rotacionar de forma automática; ficou como melhoria desejável, sem prazo.

O estado do Terraform pode conter valores sensíveis mesmo com atributos marcados, por isso o acesso ao backend de estado é restrito. Quem lê o estado lê coisa que não devia estar solta. Isso entra na revisão de acessos periódica.

## Problemas conhecidos e pendências

Lista solta do que me lembro, sem ordem de prioridade:

- O plano tem diferenças que aparecem toda vez sem significado real, geralmente em atributos que o provedor normaliza de outro jeito. Já tentei silenciar com ignorar mudanças em alguns recursos, mas isso esconde mudanças reais se alguém abusar. O melhor seria corrigir o valor no código para bater com o normalizado.
- A versão do provedor está fixada, e atualizar costuma gerar um plano grande de ajustes. Quando a atualização é feita, deve ser numa mudança só dela, sem misturar com mudança de recurso. Misturar torna impossível saber o que causou cada diferença.
- Módulos compartilhados entre ambientes precisam de cuidado ao evoluir. Uma mudança que parece pequena num módulo pode aparecer em todos os ambientes. Considero versionar os módulos para poder promover mudança de forma gradual.
- A documentação do `ledgerlark-infra` está atrasada em relação ao código. Esta nota não resolve isso. Parte do que está escrito aqui deveria virar documentação de verdade.
- Falta um jeito claro de testar mudanças de infra antes de chegar a um ambiente compartilhado. Hoje o ambiente menos crítico faz esse papel, e funciona, mas não é ideal.

### Coisas que quebraram antes

Uma aplicação que parou no meio por limite de taxa da API do provedor: o estado ficou parcialmente atualizado, e foi preciso rodar o plano de novo e deixar convergir. Nada grave, mas assustou. Desde então, mudanças grandes são divididas em partes menores quando possível.

Uma importação de recurso criado à mão no console. Alguém criou algo fora do Terraform para resolver um incidente, e depois o plano queria destruir ou duplicar. A solução foi importar o recurso para o estado e ajustar o código até o plano ficar limpo. Regra derivada: mudança de emergência feita à mão precisa ser reconciliada no código logo depois, senão vira dívida que explode no plano seguinte.

Uma variável com valor errado num ambiente novo, que quase reduziu retenção. Já contei acima. A lição é validar variáveis com restrições no próprio código, para que valores absurdos falhem cedo.

### O que eu faria a seguir

Primeiro, escrever as validações de variáveis que faltam, principalmente retenção, tamanho de armazenamento e contagem de partições, onde um valor errado custa caro. Segundo, revisar quais recursos com estado têm proteção contra destruição acidental e fechar os que não têm. Terceiro, passar esta nota e a anterior sobre o mesmo assunto por uma leitura juntas e consolidar numa só, porque duas notas soltas sobre o plano do `ledgerlark-infra` vão divergir.

Quarto, conversar com quem opera o fechamento dos clientes sobre janelas de manutenção. Hoje a informação está na cabeça de algumas pessoas. Se estiver escrita, o plano de qualquer mudança de instância ou de rede pode ser conferido contra ela sem depender de ninguém lembrar.

## Revisão de planos: checklist informal

Isto é o que eu passo na cabeça antes de aprovar ou aplicar. Não é oficial.

Primeiro: o plano foi gerado a partir do código que eu acho que foi? Confiro que a revisão do código é a esperada e que as variáveis do ambiente correto foram usadas. Parece bobo, mas já vi plano gerado no ambiente errado.

Segundo: há destruição ou substituição? Se houver, de quê? Se for recurso sem estado, como regra de rede ou papel de acesso, pondero o impacto de indisponibilidade. Se for recurso com estado, paro e converso com quem for responsável.

Terceiro: as mudanças in-place são as que eu esperava? Se aparecer um atributo que eu não toquei, investigo antes de aplicar. Muitas vezes é deriva causada por alguém que mexeu no console, e isso precisa ser decidido: o código vence ou o estado real vence?

Quarto: a mudança toca algo que afeta o fechamento dos clientes? Se sim, conferir calendário. Os clientes do Ledgerlark são times financeiros de marketplaces, e o fechamento deles tem horários em que qualquer instabilidade é problema sério, porque conciliação atrasada atrasa a revisão de divergências.

Quinto: depois de aplicar, rodar o plano de novo e esperar que ele diga que não há mudanças. Se ainda aparecer diferença, algo não convergiu ou há ruído de normalização. Em ambos os casos quero saber antes de ir embora.

Sexto: registrar o que foi feito, com o motivo, onde o time costuma registrar. Quem vier depois precisa entender por que o recurso está do jeito que está, e o histórico do repositório só diz o quê, raramente o porquê.

### Deriva

Deriva entre código e realidade é o assunto recorrente. Há alguns recursos que são alterados por processos automáticos fora do Terraform, como escalonamento ou rotação, e isso aparece no plano como mudança toda vez. O tratamento certo é dizer ao Terraform para ignorar aquele atributo específico, com um comentário explicando por quê. Ignorar sem comentário é receita para ninguém saber depois.

Para deriva causada por humanos, o caminho é conversar. Se alguém alterou algo no console por necessidade, a alteração ou entra no código ou é revertida. Deixar no limbo é pior que qualquer das duas escolhas.

### Pessoas e responsabilidades

Não tenho certeza de quem exatamente aprova mudanças em cada parte. A prática que vi: mudanças no banco e na rede passam por mais de um par de olhos, mudanças em tópicos passam por quem mantém os consumidores afetados. Se isso não estiver escrito em lugar nenhum, deveria. Fica como pendência junto das outras.

## Observações finais

Esta nota é parcial. Falta o detalhe de módulo por módulo, falta o desenho de dependências entre eles, e falta a lista de variáveis obrigatórias de cada ambiente. Tudo isso se descobre lendo o código do `ledgerlark-infra`, e não quis copiar para cá algo que vai ficar velho.

O que eu quis deixar registrado é a atitude: o plano do Terraform é a última defesa antes de a mudança chegar a dados financeiros de terceiros. Ler com calma custa pouco perto de recriar um tópico com dados retidos ou de substituir um banco. Quando algo no plano parecer estranho, a resposta correta é quase sempre parar e perguntar, não aplicar para ver o que acontece.

Se esta nota e a anterior sobre o mesmo assunto se contradisserem, confiar no código, depois no estado real, depois nas notas. E consolidar as duas numa só assim que alguém tiver uma hora livre.

Mais alguns pontos soltos que não couberam nas seções acima, anotados na pressa.

Sobre tempo de execução: o plano completo de um ambiente demora o bastante para irritar, e a tentação é usar a opção de pular a atualização do estado para ganhar tempo. Eu evito. Pular essa etapa faz o plano se basear em informação velha, e então ele pode mentir. Se a lentidão atrapalha de verdade, o caminho é restringir o plano a um módulo específico, mas isso também tem armadilha: dependências fora do alvo ficam sem avaliação, e o plano parcial pode esconder efeito colateral. Uso planos restritos só para investigar, nunca para aplicar em ambiente que importa.

Sobre nomes de recursos: renomear um recurso no código faz o Terraform entender como destruir um e criar outro. Para recurso com estado isso é desastre. O jeito certo é declarar a movimentação no código ou mover no estado, de modo que o plano mostre só a mudança de endereço. Sempre que o plano mostrar um par destruir e criar com a mesma cara, desconfio de renomeação.

Sobre módulos externos: alguns recursos vêm de módulos publicados por terceiros. Atualizar um desses módulos é como atualizar o provedor, pode trazer mudanças grandes. Também fixo a versão e atualizo em mudança separada. Leio o histórico de mudanças do módulo antes, porque às vezes há quebra de compatibilidade que o plano só revela na hora da aplicação.

Sobre observabilidade: os alertas de infraestrutura também estão no código, e mudar um recurso sem atualizar o alerta correspondente deixa o monitoramento cego. Quando o plano mexe em tópico ou banco, confiro se há alerta ligado àquele recurso e se ele continua apontando para o lugar certo. Alerta de atraso de consumo nos tópicos de liquidação é o que mais me interessa, porque atraso ali significa conciliação atrasada.

Sobre custo: o plano não mostra custo, mas toda mudança de tamanho ou de quantidade de partições e réplicas tem efeito na conta. Quando a mudança aumenta o consumo de recursos de forma relevante, aviso quem cuida do orçamento. Ninguém gosta de descobrir na fatura.

Sobre reversão: nem toda mudança pode ser revertida aplicando o código anterior. Mudanças em recursos com estado, como retenção que já apagou dados, não voltam. Antes de aplicar, penso no que acontece se der errado e se existe caminho de volta. Se não existe, o nível de cuidado sobe e a mudança ganha janela própria, com alguém de plantão sabendo que ela está acontecendo.

Sobre comunicação: avisar o time de operações financeiras só é necessário quando há risco real de impacto, mas quando há, é melhor avisar de menos antecedência do que não avisar. Uma mensagem curta dizendo o que vai mudar, quando, e o que esperar, já resolve. O ruído de avisos desnecessários também cansa, então o critério é o risco de afetar o fechamento.

Por fim, uma dúvida que ficou aberta: se vale a pena separar o estado em partes menores, por exemplo uma para dados (banco e Kafka) e outra para rede e serviços. Um estado só facilita enxergar tudo junto, mas aumenta o raio de dano de uma execução ruim e deixa o plano mais lento. Separar reduz o risco, mas cria dependência entre estados e complica a ordem de aplicação. Não decidi, e não é decisão para tomar sozinho numa nota apressada. Registro aqui só para não perder a ideia.

Também ficou na lista conversar sobre ambientes efêmeros para testar mudanças maiores de infra. A ideia é subir uma cópia reduzida do `ledgerlark-infra`, aplicar a mudança lá, rodar alguns arquivos de liquidação de exemplo pelo fluxo de conciliação e só depois levar adiante. O custo de montar isso é a objeção óbvia, e a resposta é que o custo de errar em dados financeiros costuma ser maior. Mesmo assim, é trabalho que alguém precisa assumir, e por enquanto ninguém assumiu.

É isso. Se alguém for retomar o assunto, comece pela leitura do plano de um ambiente menos crítico com calma, e use esta nota só como lembrete de onde estão as armadilhas.
