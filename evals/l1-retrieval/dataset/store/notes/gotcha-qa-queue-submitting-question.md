---
id: 01KKZRGA4EG9BD2QN412GXSH53
created: 2026-03-18T03:00-03:00
---

# qa-queue: limite de tamanho da pergunta

Pegadinha do qa-queue: se alguém envia uma pergunta com mais de 280 characters, o qa-queue rejeita e devolve o código de erro `question_too_long`. Não existe salvamento parcial. Nada é gravado, nem a parte que caberia no limite. Por isso o cliente precisa truncar o texto antes de enviar; o servidor não vai fazer isso por você.

Resumindo o contrato: limite de `280 characters` por pergunta, erro `question_too_long` quando passa disso, nenhuma gravação parcial, e a responsabilidade de truncar é do cliente.

## O que acontece

O cliente (a interface em Next.js, ou qualquer integração que fale com o backend em Phoenix) manda a pergunta pelo WebSocket. O qa-queue valida o tamanho antes de qualquer escrita no CockroachDB. Se o texto passa do limite, a resposta é o erro `question_too_long` e a pergunta nunca entra na fila de moderação. Quem está no evento vê a pergunta sumir, a não ser que a interface trate o erro e mostre uma mensagem.

Como não há registro parcial, não adianta procurar a pergunta cortada no banco depois. Ela simplesmente não existe. Isso confunde quando alguém reclama que "mandei a pergunta e ela não apareceu para o moderador".

## Por que dá problema na prática

Em eventos grandes o público cola textos longos, às vezes um parágrafo inteiro de outro lugar. O campo de texto do navegador nem sempre impõe o limite, então o erro aparece só depois do envio. Em horário de pico, com muita gente perguntando ao mesmo tempo, vários participantes tentam de novo com o mesmo texto longo e recebem o mesmo erro de novo, o que gera ruído e reclamação para os produtores do evento.

Outro ponto: contar caracteres no cliente e no servidor pode dar números diferentes se o método de contagem for diferente. Emojis e caracteres acentuados são o caso clássico. Se o cliente conta por unidades de código e o servidor por caracteres, uma pergunta perto do limite pode passar na checagem do cliente e mesmo assim ser recusada. Para ficar seguro, truncar com uma folga pequena abaixo do limite.

## Como lidar

- Truncar no cliente antes do envio, de forma que o texto fique dentro de `280 characters`.
- Mostrar um contador visível no campo de texto, para a pessoa ver quanto falta.
- Tratar o erro `question_too_long` na interface com uma mensagem clara, mantendo o texto digitado no campo para a pessoa editar, em vez de apagar tudo.
- Não repetir o envio automaticamente ao receber esse erro; o resultado seria o mesmo.
- Em integrações externas, validar o tamanho antes de chamar o qa-queue.

## Como reconhecer o sintoma

Se um produtor ou community manager diz que perguntas "somem", conferir primeiro os logs do cliente e da conexão WebSocket procurando `question_too_long`. Se o erro está lá, o problema é o tamanho e não a moderação nem a fila. Só depois disso vale investigar outras causas, como falha de conexão ou filtro de moderação.

## Pendências

Vale decidir se o servidor deveria oferecer truncamento opcional, mas hoje o comportamento é recusar tudo sem salvar nada. Qualquer mudança nisso precisa atualizar esta nota e avisar quem mantém o cliente em Next.js.
