---
id: 01K7VB78XJHW09Q2RDCB76GK2Y
created: 2025-10-18T06:43-03:00
---

# Especificação do entry-store-schema: limite da coluna Body

Nota rápida sobre o limite de tamanho da coluna Body no `entry-store-schema`, escrita para quem for mexer no armazenamento de entradas do LabNotebook Sync e precisar saber o que é permitido, onde o limite é checado e o que acontece quando alguém passa dele. O fato central é simples: a coluna Body do `entry-store-schema` pode guardar no máximo `4 MB` por entrada. Todo o resto desta nota é consequência disso ou contexto para aplicar a regra sem surpresa.

O limite vale por entrada, não por lote, não por usuário e não por notebook. Cada entrada do caderno eletrônico tem o seu próprio Body, e cada um desses valores precisa caber em `4 MB`. Uma entrada com o corpo maior que isso não deve ser gravada no SQL Server, nem parcialmente, nem truncada em silêncio.

## O que a regra diz

A coluna Body do `entry-store-schema` guarda o conteúdo principal de uma entrada de caderno: o texto escrito pelo cientista, as anotações estruturadas e as referências a dados de instrumento que já foram resolvidas no momento da gravação. O teto é `4 MB` por entrada. Se alguém perguntar qual é o tamanho máximo do Body de uma entrada, a resposta é `4 MB`, e essa resposta não depende do tipo de entrada, do projeto ou do perfil de quem escreve.

Pontos que ficam claros com a regra:

- O limite é um teto, não um valor típico. A maioria das entradas é muito menor, e o esquema não reserva espaço antecipado para chegar perto do teto.
- O limite é sobre o conteúdo do Body. Metadados da entrada, como autor, carimbo de tempo, estado de revisão e vínculos de auditoria, ficam em outras colunas e não entram nessa conta.
- Conteúdo que ultrapassa o teto não é cortado para caber. Quem escreve precisa receber uma recusa explícita, porque truncar um registro de laboratório em silêncio seria um problema de integridade e de conformidade.
- Anexos grandes, como arquivos brutos de instrumento, imagens e exportações, não pertencem ao Body. Eles vão para o Azure Blob Storage, e o Body guarda só a referência a eles.

A razão do teto é prática. Entradas de caderno são lidas e reescritas com frequência pelo sincronizador, e cada revisão gera uma nova versão para a trilha de auditoria. Um Body sem limite faria o custo de armazenamento, de leitura e de comparação de versões crescer sem controle, e deixaria o tráfego pelo RabbitMQ imprevisível, já que mensagens de sincronização carregam o conteúdo da entrada ou parte dele. Com um teto conhecido dá para dimensionar filas, tempos de limite e índices com segurança.

## Onde o limite é aplicado

O limite deve ser verificado em mais de uma camada. Confiar só em uma delas é pedir para ter um registro inválido passando por outro caminho.

Primeiro, na borda da aplicação em C#. Antes de montar o comando de gravação, o serviço que recebe a entrada mede o tamanho do Body em bytes, já na codificação em que ele será armazenado, e compara com o teto de `4 MB`. É nesse ponto que a recusa deve acontecer, com uma mensagem que diga ao usuário que o conteúdo é grande demais e sugira mover o material volumoso para um anexo. Medir em caracteres não serve: texto com acentos, símbolos científicos e caracteres fora do alfabeto básico ocupa mais de um byte por caractere, e a contagem em caracteres subestima o tamanho real.

Segundo, no caminho do sincronizador. Entradas que chegam de fora, por exemplo vindas de um instrumento ou de outro cliente, passam por consumidores do RabbitMQ antes de ir ao banco. O consumidor aplica a mesma checagem. Se a mensagem trouxer um Body acima do teto, ela não deve ser reenfileirada indefinidamente: vai para o tratamento de mensagens com falha, com o motivo registrado, para que alguém possa olhar. Reenfileirar uma mensagem que nunca vai caber só gera ruído e esconde o problema real.

Terceiro, no próprio SQL Server. O esquema deve ter uma restrição que impeça a gravação de um Body acima do teto, de modo que mesmo um cliente que contorne as camadas acima não consiga inserir um valor inválido. A restrição é a última linha de defesa, não a primeira. Se ela disparar em produção, isso indica que alguma checagem anterior falhou ou foi esquecida, e vale investigar a origem em vez de só tratar o erro.

O trecho abaixo resume a regra como comentário de esquema, só para ficar ao lado da definição da coluna:

```sql
-- entry-store-schema: coluna Body, no máximo 4 MB por entrada
```

O comentário não substitui a restrição real. Ele existe para que quem abrir o script de esquema encontre o limite sem precisar procurar nesta nota.

## Como lidar com conteúdo que não cabe

Quando uma entrada precisa de mais espaço do que o Body permite, a resposta não é aumentar o teto. A resposta é separar o conteúdo.

O caminho recomendado:

- Texto narrativo, procedimentos e conclusões ficam no Body, que é o lugar certo para o que o cientista escreve e para o que o responsável de conformidade vai revisar.
- Dados volumosos de instrumento, séries longas, imagens e arquivos binários vão para o Azure Blob Storage. No Body fica uma referência estável a cada blob, e a trilha de auditoria registra qual versão do blob estava ligada a qual versão da entrada.
- Se a entrada for naturalmente longa, como um relatório extenso, o certo é dividi-la em entradas relacionadas, ligadas entre si, e não forçar tudo num único Body no limite do teto.

Um cuidado que merece atenção: o vínculo entre o Body e o blob precisa ser imutável do ponto de vista da auditoria. Se o blob puder ser substituído sem deixar rastro, a entrada passa a apontar para um dado diferente do que o cientista viu ao assinar. Por isso a referência deve identificar o conteúdo exato, e não apenas um nome de arquivo que possa ser reutilizado.

Para quem cuida da interface, a mensagem de recusa precisa ser útil. Dizer apenas que houve um erro de gravação não ajuda ninguém. O texto deve dizer que o corpo da entrada passou do limite de `4 MB` e indicar a alternativa, que é anexar o material pesado. Isso reduz chamados para o suporte e evita que o usuário tente de novo a mesma coisa.

## Efeitos em auditoria, testes e manutenção

A trilha de auditoria é o motivo pelo qual esse limite não pode ser tratado de forma frouxa. O LabNotebook Sync existe para manter um registro confiável do que foi escrito, quando e por quem. Cada versão de uma entrada é preservada. Se o Body pudesse crescer sem limite, o volume de versões guardadas cresceria junto, e a comparação entre versões, que os responsáveis de conformidade usam, ficaria lenta ou inviável. O teto de `4 MB` mantém esse custo previsível.

Também importa o que acontece com uma recusa. Uma gravação recusada por tamanho não deve criar versão na trilha de auditoria, porque nada foi gravado. Mas a tentativa em si pode ser registrada como evento operacional, separado da trilha de versões, para que se possa ver depois que alguém tentou gravar algo grande demais. Essa separação evita que a trilha de versões se encha de entradas que nunca existiram.

Sobre testes, vale cobrir pelo menos estes casos:

- Um Body exatamente no teto de `4 MB` deve ser aceito. O limite é inclusivo, e um erro de um byte na comparação é o bug mais comum nesse tipo de regra.
- Um Body um byte acima do teto deve ser recusado, e nada deve ser gravado.
- Um Body com muitos caracteres multibyte, perto do teto, para garantir que a medição é em bytes e não em caracteres.
- Uma mensagem do RabbitMQ com Body acima do teto, para confirmar que ela vai para o tratamento de falha e não entra em laço de reentrega.
- Uma tentativa de inserção direta no SQL Server acima do teto, para confirmar que a restrição do esquema dispara mesmo sem passar pela aplicação.

Sobre manutenção, algumas regras para quem for alterar este esquema no futuro:

- Mudar o teto do Body é uma mudança de contrato. Afeta o esquema, o serviço em C#, os consumidores do RabbitMQ e a documentação. Não deve ser feita só numa camada.
- Reduzir o teto exige verificar antes se existem entradas já gravadas acima do novo valor. Entradas existentes não podem ser invalidadas nem truncadas retroativamente, porque fazem parte de um registro auditado.
- Aumentar o teto exige avaliar o efeito no tráfego das filas, nos limites de mensagem do RabbitMQ e no tempo de leitura das versões, antes de mexer no banco.
- Qualquer migração que toque na coluna Body deve preservar o conteúdo byte a byte. Conversões de codificação ou normalização de espaços em branco alteram o conteúdo e podem invalidar assinaturas e somas de verificação de versões antigas.

Por fim, uma observação sobre dúvidas comuns. Se alguém perguntar se o limite inclui os anexos, a resposta é não: os anexos ficam no Azure Blob Storage e só a referência entra no Body. Se perguntarem se o limite é por entrada ou por notebook, é por entrada. E se perguntarem qual é o valor, é `4 MB` para o Body de cada entrada no `entry-store-schema`. Qualquer divergência entre o que o banco aceita e o que a aplicação aceita deve ser tratada como defeito, e a correção vai na camada que estiver errada, nunca no teto.

## Pendências e pontos em aberto

Algumas coisas ainda não estão decididas e não devem ser assumidas por quem ler esta nota:

- A forma exata da restrição no SQL Server, e se ela mede bytes do valor armazenado ou do valor lógico, precisa ser confirmada no script de esquema antes de qualquer mudança. Esta nota descreve a intenção, não o texto do script.
- O formato da mensagem de recusa mostrada ao usuário ainda pode mudar. O que não muda é o conteúdo mínimo: dizer que o limite foi excedido e apontar para o uso de anexos.
- O destino exato das mensagens do RabbitMQ rejeitadas por tamanho depende da topologia de filas em uso. O requisito é só que elas não entrem em reentrega infinita e que o motivo fique registrado.
- Ainda falta alinhar com a equipe de conformidade como registrar, na trilha operacional, as tentativas recusadas, e por quanto tempo guardá-las.

Se alguma dessas pendências for resolvida, atualize esta nota no lugar, em vez de criar outra sobre o mesmo assunto, e remova daqui o que deixar de valer.
