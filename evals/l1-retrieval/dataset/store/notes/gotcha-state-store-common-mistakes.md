---
id: 01KV1EY7K1370DF9C1C0Q9WT5N
created: 2026-06-13T18:41-03:00
---

# Erros comuns com o state-store

Anotação rápida dos tropeços mais comuns com o state-store no PatchPilot. Nada aqui é erro exato nem valor fixo; é o que costuma dar errado quando alguém mexe nele com pressa.

## Tratar como cache descartável

Muita gente acha que o state-store é só cache e pode ser apagado quando algo estranho aparece. Ele guarda o histórico do que já foi tentado em cada repositório. Apagar faz o PatchPilot reabrir PRs que já tinham sido fechados ou ignorados.

## Misturar estado com configuração

Colocar preferências de repositório dentro do state-store parece prático. Depois ninguém sabe o que é decisão humana e o que é resultado de execução. Configuração fica versionada fora dele; o state-store guarda só o que a ferramenta observou.

## Escrita concorrente sem cuidado

Vários jobs do GitHub Actions podem tocar no mesmo arquivo ao mesmo tempo. SQLite aguenta leitores em paralelo, mas escritores se bloqueiam. Quem ignora isso vê travas intermitentes e conclui que o banco está corrompido, quando era só contenção.

## Compartilhar o arquivo entre runners

Montar o mesmo arquivo em runners diferentes por volume de rede é tentador. Bloqueio de arquivo em sistema de arquivos de rede é pouco confiável. O resultado é estado perdido ou inconsistente, e difícil de reproduzir.

## Esquecer que o container é efêmero

No Docker, se o state-store ficar dentro da camada gravável do container, ele some quando o container é recriado. Parece que o PatchPilot "esqueceu" tudo. É preciso um volume persistente e conferir que ele está de fato montado.

## Migrações feitas na mão

Alterar o esquema direto no banco para destravar algo é o caminho mais rápido e o mais perigoso. Outra pessoa roda a versão normal e as migrações divergem. Mudança de esquema passa pelo mecanismo de migração do projeto.

## Rodar migração sem backup

Antes de qualquer migração, copie o arquivo. Parece exagero até a primeira migração parcial. Copiar com o banco em uso, sem cuidado, também gera cópia inconsistente.

## Cópia de arquivo com o banco aberto

Copiar só o arquivo principal ignora os arquivos auxiliares do modo de journal. A cópia pode vir sem escritas recentes. Use a API de backup do SQLite ou pare os escritores antes.

## Ignorar o crescimento

O histórico cresce com o tempo, principalmente com muitos repositórios. Sem política de retenção, o arquivo incha e as consultas ficam lentas. Ninguém percebe até o job começar a estourar o tempo.

## Consultas sem índice

Filtros por repositório e por pacote são os mais usados. Adicionar uma consulta nova sem pensar em índice funciona bem no teste local, com pouco dado, e fica lenta em produção.

## Guardar segredos no estado

Tokens e credenciais não devem entrar no state-store, nem em campos de log ou de erro. O arquivo circula em backups e artefatos de CI, e vaza com eles.

## Subir o arquivo como artefato

Enviar o banco como artefato do workflow para depuração é comum. Se o repositório for público ou o artefato ficar acessível demais, o histórico interno do PatchPilot fica exposto. Confira o que vai dentro antes.

## Confiar em horário local

Gravar datas em fuso local e comparar em outro lugar quebra regras de reabertura e de espera. Guarde tudo de forma uniforme e converta só na exibição.

## Testes que usam o estado real

Rodar testes contra o state-store de verdade contamina o histórico. Teste usa banco temporário e novo, descartado no final. Também vale não reaproveitar o banco de um teste no seguinte.

## Transações longas

Manter uma transação aberta enquanto se espera a API do GitHub ou um teste direcionado bloqueia os outros escritores. Busque os dados primeiro, depois abra a transação, grave e feche.

## Tratar falha de gravação como aviso

Se a gravação falha e o fluxo segue, o PatchPilot perde o rastro do que fez e repete o trabalho na próxima rodada. Falha de gravação deve parar a etapa e aparecer com clareza.

## Antes de culpar o banco

Na maioria das vezes o problema é volume não montado, concorrência ou migração fora de ordem. Verifique isso primeiro e só depois suspeite de corrupção.
