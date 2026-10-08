---
id: 01KQ15C77HT80YW0GYMRCH4Y06
created: 2026-04-24T22:51-03:00
---

# room-constraints: equipamentos na tabela room_equipment

Esta nota substitui a nota anterior "room constraints stores equipment". Valor novo: o `room-constraints` agora guarda os equipamentos da sala na tabela de junção `room_equipment`, que substitui a coluna JSON usada antes, porque as clínicas precisam filtrar salas por equipamento direto em SQL.

## Decisão

O `room-constraints` armazena equipamento na tabela de junção `room_equipment`. A coluna JSON que existia antes na tabela de salas deixa de ser a fonte da verdade. Cada linha de `room_equipment` liga uma sala a um tipo de equipamento, e uma sala pode ter vários equipamentos. Quem ler só esta nota deve entender: equipamento de sala = linhas em `room_equipment`, não mais JSON.

## Por que mudamos

As clínicas precisam filtrar salas por equipamento (por exemplo, sala com um aparelho específico para um tipo de consulta). Com JSON no MySQL esse filtro fica lento, depende de funções específicas e é difícil de indexar. Com a tabela de junção o filtro vira um JOIN comum, com índice, e o agendador consegue restringir as salas candidatas já na consulta, antes de montar os horários.

## Alternativas descartadas

- Manter a coluna JSON e filtrar com funções JSON do MySQL: funciona, mas não indexa bem e deixa as consultas do Rails mais feias.
- Filtrar em Ruby depois de carregar as salas: carrega dados demais e piora quando a clínica tem muitas salas.

## Impacto

- Models do Rails: a sala passa a ter associação com os equipamentos pela tabela de junção; código que lia o JSON precisa ser trocado.
- Migração: os dados existentes do JSON têm que ser copiados para `room_equipment` antes de remover a coluna. Conferir se os jobs do Sidekiq que usam dados de sala não leem mais o JSON.
- Integração HL7 FHIR: se algum recurso exportado montava equipamento a partir do JSON, ele deve ler da nova tabela.
- Deploy no Heroku: rodar a migração antes de subir o código novo, para não ter janela em que o código procura a tabela e ela não existe.

## Pendências

- Confirmar que nenhum relatório antigo ainda lê a coluna JSON.
- Depois da migração validada, remover a coluna JSON em uma mudança separada.
- Se surgir outra nota dizendo que o equipamento fica em JSON, ela está desatualizada: vale esta aqui.
