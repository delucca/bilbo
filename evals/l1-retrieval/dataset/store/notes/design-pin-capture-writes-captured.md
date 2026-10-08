---
id: 01K6M2S3HNDEFCK5SQRF6AVPB1
created: 2025-10-03T00:45-03:00
sources:
  - "code: app/src/main/kotlin/com/parcelpin/capture/CameraController.kt"
---

# Design do pin-capture-app: captura de fotos e armazenamento offline

Nota de design do `pin-capture-app`, o app Android que o entregador usa para registrar a prova de entrega no ParcelPin. O ponto central: a foto é gravada em disco local antes de qualquer tentativa de envio. A rede nunca fica no caminho da captura. Esta nota explica o porquê, como o fluxo é montado e o que costuma dar errado.

## Nome do componente

O componente se chama `pin-capture-app`. O nome antigo era `podroid`. Ainda dá para encontrar `podroid` em issues antigas, em mensagens de commit, em nomes de branch e em alguns comentários soltos no código. Quem achar esse nome deve ler como o mesmo componente, o app de captura. Não existe um segundo projeto chamado `podroid` ao lado dele.

Se for abrir issue, escrever documentação ou nomear algo novo, use `pin-capture-app`. Nas buscas no histórico, vale procurar pelos dois nomes, senão se perde contexto de decisões antigas. A troca de nome foi só de nomenclatura. A arquitetura descrita abaixo vale para o componente com qualquer um dos dois nomes.

## Decisão principal: gravar primeiro, enviar depois

O `pin-capture-app` grava as fotos capturadas no diretório privado do app, `files/proofs/`, usando `CameraX 1.3.4`, antes de qualquer tentativa de envio. Essa ordem é proposital e não deve ser invertida.

Motivos, em ordem de importância:

- O entregador trabalha em lugares com sinal ruim: elevador, garagem, condomínio com parede grossa, zona rural. Se a captura dependesse de rede, a prova se perderia justamente quando mais faz falta.
- A foto em disco é a fonte da verdade. Se o app morrer, o celular reiniciar ou a bateria acabar no meio do envio, o arquivo continua lá e o envio recomeça depois.
- Separar captura de envio deixa cada etapa com um único motivo para falhar. Erro de câmera é um problema. Erro de rede é outro, e tratado em outro lugar.

O diretório é privado ao app. Outros apps não leem esses arquivos e a galeria do aparelho não mostra as fotos. Isso é desejado: a prova de entrega não deve se misturar com as fotos pessoais do motorista, e o entregador não deve apagá-las por engano ao limpar a galeria.

## Fluxo de captura

O fluxo, do toque do entregador até a foto segura em disco:

1. O entregador abre a tela de entrega de uma parada e aciona a câmera.
2. O app liga a pré-visualização e o caso de uso de captura de imagem do CameraX.
3. Ao tirar a foto, o CameraX grava o arquivo direto em `files/proofs/`, sem passar por memória compartilhada nem por pasta pública.
4. Só depois de o arquivo estar completo em disco o app registra a prova no banco local, com referência ao arquivo e à parada.
5. A interface mostra a prova como capturada e pendente de envio. O entregador pode seguir para a próxima parada sem esperar.

O passo quatro vem depois do três de propósito. Se o registro existisse sem o arquivo, o envio falharia mais tarde por um motivo difícil de entender. Um arquivo sem registro, ao contrário, é um resíduo inofensivo que a limpeza periódica remove.

A tela de captura não faz chamada de rede. Se alguém precisar de algum dado remoto nessa tela, deve levá-lo para antes, no carregamento da parada, e deixar a captura intocada.

## Envio e sincronização

O envio é uma etapa separada, que roda em segundo plano e lê o que está pendente. Cada prova pendente tem o arquivo em `files/proofs/` e um registro local com o estado. A sincronização funciona assim, em termos gerais:

- Um trabalho em segundo plano, agendado com as ferramentas do Android Jetpack, procura provas pendentes e tenta enviar uma a uma.
- A imagem vai para o armazenamento do Firebase e os metadados da entrega são gravados no Cloud Firestore.
- Os dados estruturados da prova seguem um esquema definido em Protocol Buffers, compartilhado com o backend. Mudou o esquema, os dois lados precisam ser atualizados com cuidado para manter a compatibilidade.
- Quando o envio termina com sucesso, o registro local é marcado como enviado. O arquivo só pode ser apagado depois dessa confirmação.
- Se falhar, a prova continua pendente e o trabalho tenta de novo mais tarde, com espera crescente entre as tentativas.

O Cloud Firestore também tem cache offline próprio, mas não confiamos nele para a imagem. A foto é um arquivo grande e vive em `files/proofs/`. O cache do Firestore cuida apenas dos metadados, e mesmo isso passa antes pelo registro local do app.

O envio deve ser idempotente. Se o app enviar a mesma prova duas vezes, por exemplo porque a confirmação se perdeu na rede, o backend não pode criar duas entregas. Por isso cada prova carrega um identificador gerado no aparelho, no momento da captura, e o servidor usa esse identificador para reconhecer repetições.

## Armadilhas conhecidas

Coisas que já causaram ou podem causar problema:

- **Apagar cedo demais.** Apagar o arquivo de `files/proofs/` antes da confirmação do envio faz perder a prova de forma irrecuperável. Qualquer rotina de limpeza precisa checar o estado no registro local, não só a idade do arquivo.
- **Armazenamento cheio.** Com o celular quase sem espaço, a gravação pode falhar. O app precisa avisar o entregador na hora, com mensagem clara, e não fingir que a foto foi salva. Uma captura que falhou em silêncio é pior que uma que mostra erro.
- **Atualização do CameraX.** A versão fixada é `CameraX 1.3.4`. Trocar de versão muda o comportamento em alguns aparelhos, principalmente modelos mais baratos e antigos, que são comuns entre os entregadores. Qualquer atualização deve ser testada em aparelhos reais de gama baixa antes de ir para produção.
- **Registro sem arquivo.** Se o sistema limpar o armazenamento do app ou houver restauração de backup, pode sobrar registro apontando para arquivo inexistente. O envio precisa tratar isso como erro permanente da prova e marcar o caso para revisão, em vez de repetir para sempre.
- **Orientação e rotação.** Girar o aparelho durante a captura recria a tela. O estado da captura em andamento não pode ficar preso à tela, senão a foto some ou é duplicada.
- **Nome antigo.** Quem procurar só por `pin-capture-app` no histórico pode não achar discussões de quando o componente ainda era `podroid`.

## O que ainda está em aberto

Pontos que não estão decididos e merecem conversa antes de mexer:

- Política de retenção: por quanto tempo manter localmente uma prova já enviada. Hoje a tendência é apagar logo depois da confirmação, para poupar espaço, mas o suporte às vezes pede a foto de volta quando há disputa de entrega.
- Compressão: se vale reduzir a imagem antes do envio para economizar dados móveis do entregador, sem perder legibilidade da prova. Qualquer mudança aqui deve preservar o original em disco até o envio ser confirmado.
- Visibilidade para o entregador: mostrar com mais clareza quantas provas estão pendentes, para ele saber se pode desligar o celular ou fechar o app com tranquilidade.
- Provas muito antigas que nunca foram enviadas: definir se o app insiste, avisa o entregador ou escala para o suporte.

Enquanto isso não for decidido, a regra de ouro continua: a foto vai primeiro para `files/proofs/`, o envio vem depois, e nada é apagado antes da confirmação.
