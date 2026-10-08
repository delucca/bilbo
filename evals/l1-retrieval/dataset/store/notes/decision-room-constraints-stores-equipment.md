---
id: 01JXQNT1H2HGC2DYEX87VHBFZX
created: 2025-06-14T13:54-03:00
---

# room-constraints: requisitos de equipamento em coluna JSON, sem tabela de junção

Decidimos que o `room-constraints` guarda os requisitos de equipamento numa coluna JSON chamada `required_equipment`, em vez de uma tabela de junção. O motivo é simples: nenhuma sala lista mais de 5 itens. Com uma lista tão curta, a tabela de junção só traria joins, migrações e código extra, sem nenhum ganho de consulta. Esta nota registra a decisão, o raciocínio e o que precisa mudar se a premissa deixar de valer.

## Contexto

O ClinicSlotter agenda consultas ambulatoriais respeitando a disponibilidade dos clínicos e as restrições de sala. Quem usa é a recepção de clínicas pequenas. Cada tipo de consulta pode exigir equipamento específico, por exemplo uma maca ajustável, um aparelho de ultrassom portátil ou um otoscópio fixo. A sala, por sua vez, tem o equipamento que de fato possui. O `room-constraints` cruza as duas listas e diz ao solver quais salas servem para um determinado pedido de horário.

A pergunta de modelagem era onde guardar essa lista. A resposta clássica em Rails com MySQL seria uma tabela de equipamentos, outra de salas e uma terceira ligando as duas. Discutimos isso e resolvemos não seguir esse caminho.

## Decisão

O `room-constraints` usa a coluna JSON `required_equipment` para guardar a lista de equipamentos exigidos. A coluna fica no próprio registro que descreve a restrição, e o valor é um array simples de identificadores de equipamento, em texto. Não existe tabela de junção para isso e não vamos criar uma agora.

Resumindo para quem lê só esta nota: equipamento no `room-constraints` mora em `required_equipment`, é JSON, e a razão é que nenhuma sala lista mais de 5 itens.

## Por que não uma tabela de junção

A tabela de junção resolve bem relações grandes, com muitos itens por lado e consultas que partem do item para as salas. Aqui o cenário é o contrário. As listas são minúsculas, os dados são lidos quase sempre junto com a sala, e ninguém precisa perguntar quais salas têm tal item com performance crítica.

Os custos que evitamos com a coluna JSON:

- uma migração a menos e um modelo ActiveRecord a menos para manter;
- nenhum join extra na consulta que o solver faz a cada tentativa de encaixe;
- menos risco de N+1 acidental quando a tela da recepção lista salas;
- escrita atômica: a lista inteira muda numa única atualização do registro.

Para clínicas pequenas, a simplicidade pesa mais do que a normalização completa.

## Como o dado é lido e escrito

O Rails trata a coluna JSON como um atributo normal, devolvendo um array de strings. A leitura acontece junto com o carregamento da restrição, sem consulta adicional. Na escrita, a aplicação sempre substitui a lista inteira, nunca altera um elemento no lugar. Isso evita condições de corrida entre dois usuários editando a mesma sala ao mesmo tempo, porque o último a salvar vence de forma previsível.

Os jobs do Sidekiq que recalculam disponibilidade também leem `required_equipment` como array comum. Eles não dependem de nenhum tipo especial do MySQL, então o comportamento é o mesmo em desenvolvimento e na produção no Heroku.

## Validações

Como o banco não impõe estrutura dentro do JSON, a validação fica no modelo. As regras que mantemos:

- o valor tem de ser um array, nunca um objeto nem texto solto;
- cada elemento é uma string não vazia;
- não pode haver elementos repetidos;
- o tamanho da lista respeita o limite de 5 itens que motivou a decisão.

O último ponto é importante: o limite não é só uma observação histórica, ele é a premissa que sustenta o desenho. Se o modelo deixar passar listas maiores, a decisão perde a base sem ninguém perceber.

## Consultas e índices

O MySQL não indexa bem o conteúdo de uma coluna JSON sem um índice funcional ou uma coluna gerada. Não criamos nenhum. A filtragem por equipamento acontece em memória, depois que as salas candidatas da clínica foram carregadas. Como o número de salas por clínica é pequeno, o custo é desprezível.

Se um dia alguém precisar buscar no banco todas as salas que têm determinado equipamento, a opção a avaliar primeiro é uma coluna gerada com índice, antes de qualquer tabela nova. Só vale pensar em tabela de junção se a consulta partir do equipamento e virar um caminho quente.

## Integração com FHIR

O ClinicSlotter conversa com sistemas externos por HL7 FHIR. Equipamentos e locais aparecem nesses recursos de forma própria, e o mapeamento é feito na borda da integração. Os identificadores guardados em `required_equipment` são os do nosso domínio, não os do FHIR. A tradução acontece nos adaptadores, de modo que a escolha de armazenamento interno não vaza para o contrato externo. Trocar JSON por tabela depois não mudaria o que os parceiros enxergam.

## Relação com o solver

O solver consome o resultado do `room-constraints` para decidir se um horário cabe numa sala. A nota [[slot-solver-must-return-revised]] trata do contrato de retorno do solver e é leitura complementar. O ponto de contato aqui é só este: o solver espera receber do `room-constraints` uma resposta pronta, sem precisar conhecer como o equipamento está armazenado. Por isso a mudança de armazenamento fica contida dentro do componente.

## Riscos e gatilhos para rever

A decisão vale enquanto a premissa valer. Os sinais de que é hora de reabrir o assunto:

- aparece uma sala com mais de 5 itens e a regra de validação precisa ser afrouxada;
- surge a necessidade de guardar atributos por item, como quantidade, estado de manutenção ou validade de calibração;
- relatórios passam a cruzar equipamento com sala em grande volume;
- outro componente quer reutilizar a mesma lista de equipamentos como cadastro compartilhado.

Qualquer um desses casos já pede entidade própria. Nesse momento a migração seria ler o JSON, criar os registros de equipamento e preencher a junção, mantendo a coluna por um tempo como leitura de segurança.

## Para quem for mexer nisso

Antes de propor tabela de junção, confira se a premissa mudou de fato. Se não mudou, mantenha `required_equipment` como está. Ao adicionar testes, cubra a validação de formato e de tamanho, e um caso com a lista vazia, que significa que a restrição não exige equipamento nenhum. Se mudar a decisão, atualize esta nota em vez de criar outra, e registre o que levou à mudança.
