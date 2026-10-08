---
id: 01JQZKE3B5BZ8177APBEGANKZ1
created: 2025-04-04T02:12-03:00
---

# Especificação do ladder-planner (rungsmith)

O `ladder-planner` decide quais degraus (rungs) entram na escada de bitrate adaptativo de cada vídeo enviado ao ReelForge. Ele recebe as propriedades do arquivo de origem e devolve a lista de degraus que o transcodificador baseado em FFmpeg deve produzir. Depois disso, o empacotamento HLS usa essa lista para montar a playlist mestra. O codinome interno do componente é `rungsmith`; aparece em logs antigos, em nomes de branches e na conversa da equipe, mas o nome oficial no código e nesta documentação é `ladder-planner`.

A regra central: o `ladder-planner` deve `never upscale`. Qualquer degrau cuja altura seja maior que a altura da origem é descartado da escada. Esta nota fixa o comportamento esperado, o motivo e o que fica de fora.

## Contexto

O ReelForge é usado por equipes de operações de mídia em publishers independentes. Essas equipes sobem material de qualidades muito variadas: de capturas de celular antigas a masters em alta resolução. Uma escada fixa, igual para todos os vídeos, gera degraus inúteis quando a origem é pequena. O planner existe para adaptar a escada ao que a origem realmente tem.

O componente é escrito em Rust e roda como uma etapa dentro do fluxo orquestrado pelo AWS Step Functions. A origem fica no AWS S3, e o planner só lê metadados já extraídos; ele não baixa nem decodifica o vídeo.

## Regra de não ampliar

Ampliar (upscale) um vídeo não acrescenta detalhe. Só aumenta o tamanho do arquivo, gasta tempo de transcodificação e dá ao player uma opção de qualidade que não existe de verdade. Por isso a regra é simples e sem exceções configuráveis:

- Compare a altura de cada degrau candidato com a altura da origem.
- Se a altura do degrau for maior que a da origem, o degrau sai da escada.
- Se for igual ou menor, o degrau fica.

A comparação é feita pela altura, não pela largura, porque a escada é definida em alturas. Vídeos verticais e anamórficos exigem atenção: a altura usada é a altura de exibição da origem, depois de aplicar rotação e proporção de pixel quando houver.

## Entrada e saída

A entrada é um conjunto de metadados da origem (altura, largura, taxa de quadros, presença de áudio) mais o template de escada configurado para o publisher. O template lista os degraus candidatos com altura e bitrate alvo.

A saída é uma lista ordenada de degraus, do menor para o maior, já filtrada pela regra acima. Cada item leva altura, largura calculada mantendo a proporção, bitrate alvo e parâmetros gerais de codificação para o FFmpeg.

Exemplo ilustrativo do formato de saída, só com a estrutura e sem valores reais de produção:

```json
{
  "planner": "ladder-planner",
  "codename": "rungsmith",
  "rule": "never upscale",
  "rungs": []
}
```

## Casos de borda

- Origem menor que todos os degraus do template: a escada ficaria vazia. Nesse caso o planner mantém um único degrau na resolução da origem, com bitrate escolhido pelo template, em vez de falhar. Assim o vídeo sempre tem pelo menos uma saída reproduzível.
- Origem com altura exatamente igual a um degrau: o degrau é mantido, pois não há ampliação.
- Origem com altura entre dois degraus: o degrau maior é descartado e o menor é mantido. O planner não cria um degrau intermediário só para igualar a origem, a menos que o template mande.
- Metadados ausentes ou inconsistentes: o planner recusa o plano e devolve erro para a etapa anterior, para o Step Functions tratar a falha. Ele não adivinha a altura.

## Integração com o fluxo

O Step Functions chama o planner depois da etapa de inspeção da origem e antes da transcodificação. A lista de degraus vira a entrada de um mapa de execução paralela, com um trabalho de FFmpeg por degrau. As saídas são gravadas no S3 e o empacotador HLS gera as playlists por degrau e a playlist mestra.

Como o planner é determinístico, o mesmo par origem e template sempre produz a mesma escada. Isso ajuda a reexecutar etapas com falha sem mudar o resultado.

## Decisões e pendências

- A regra de não ampliar não tem opção para desligar. Se alguém pedir uma escada com ampliação, a resposta é recusar no nível do planner, não criar uma flag.
- Os nomes `rungsmith` e `ladder-planner` apontam para o mesmo componente. Em código novo, métricas e documentação, usar `ladder-planner`. O codinome pode aparecer em busca de logs antigos.
- Pendente: definir como tratar origens com taxa de quadros muito alta ao escolher bitrates alvo. Hoje o planner só olha a altura para filtrar; ajuste de bitrate por taxa de quadros fica para uma revisão futura.
- Pendente: documentar no template do publisher quais degraus são obrigatórios e quais são opcionais, para o caso de origem pequena ficar mais previsível.

## O que o planner não faz

Ele não decodifica nem transcodifica vídeo, não escolhe codec por dispositivo e não monta playlists HLS. Também não corta, não recompõe e não reduz ruído. Essas tarefas pertencem a outras etapas do fluxo. O planner só responde a uma pergunta: quais degraus fazem sentido para esta origem.
