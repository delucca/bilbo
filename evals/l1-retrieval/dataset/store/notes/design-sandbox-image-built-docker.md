---
id: 01K4RBJD0E6SEAN6J2GECGWQCQ
created: 2025-09-09T20:04-03:00
sources:
  - "code: docker/sandbox.Dockerfile"
---

# Design do sandbox-image

O sandbox-image é a imagem Docker em que o PatchPilot roda os testes direcionados de cada pull request de atualização de dependência. Esta nota registra como ela é montada, por que é pequena e o que não deve entrar nela. A base é `node:22-bookworm-slim`, e a imagem instala somente o git e os gerenciadores de pacotes. Todo o resto vem de fora, na hora da execução.

Escrevi isto com pressa depois de discutir o assunto de novo. A ideia é que a próxima pessoa (ou agente) não precise refazer a conversa.

## Objetivo do sandbox-image

O PatchPilot abre pull requests que sobem versões de dependências em muitos repositórios. Antes de alguém olhar para o PR, ele roda um conjunto reduzido de testes para dizer se a atualização quebra algo. Essa execução precisa de um ambiente isolado, repetível e barato de subir. O sandbox-image é esse ambiente.

O que ele precisa fazer:

- clonar o repositório alvo e trocar de branch;
- instalar as dependências do projeto com o gerenciador que o projeto usa;
- rodar o comando de teste escolhido pelo PatchPilot;
- devolver código de saída e logs para quem chamou.

O que ele não faz: decidir quais testes rodar, abrir PR, guardar estado. Isso fica no processo principal em TypeScript e no banco SQLite.

## Base: node:22-bookworm-slim

A imagem parte de `node:22-bookworm-slim`. Escolhemos essa base por três motivos práticos.

Primeiro, o PatchPilot é todo Node.js, então o runtime já vem pronto e na linha de versão que usamos em desenvolvimento. Segundo, a variante slim do Debian bookworm é bem menor que a completa, o que reduz o tempo de pull nos runners do GitHub Actions. Terceiro, é Debian, então o comportamento de bibliotecas nativas é o que a maioria dos projetos de plataforma espera, ao contrário de imagens baseadas em musl, que já nos deram surpresa com módulos nativos.

Se alguém quiser trocar a base, a pergunta certa é se os repositórios alvo ainda compilam seus módulos nativos sem mudança. Trocar só para ganhar espaço não vale o risco.

## O que é instalado

Sobre a base, o sandbox-image instala só duas coisas: o git e os gerenciadores de pacotes. O git é necessário para clonar e para os projetos que dependem de dependências apontadas para repositórios git. Os gerenciadores de pacotes são necessários para instalar as dependências de cada projeto do jeito que o projeto espera.

Não há compilador, nem ferramentas de depuração, nem editor, nem cliente de banco. A lista é curta de propósito. Cada pacote a mais é superfície de ataque, tempo de build e motivo para a imagem divergir entre ambientes.

```dockerfile
# sandbox-image: base mínima
FROM node:22-bookworm-slim
# aqui entram apenas o git e os gerenciadores de pacotes
```

O bloco acima é só o esqueleto. O Dockerfile real tem as camadas de instalação, mas a regra é a mesma: nada além do git e dos gerenciadores.

## Por que tão enxuta

Uma imagem pequena ajuda em quatro frentes. O pull é rápido, então o PR recebe resultado mais cedo. O cache de camadas do Docker funciona melhor quando há poucas camadas e elas mudam pouco. A auditoria é mais simples, porque dá para ler a lista inteira do que está instalado. E o comportamento é previsível: se um teste passa no sandbox-image, a causa de uma falha futura quase nunca é uma ferramenta de sistema que apareceu ou sumiu.

O custo é que alguns projetos vão precisar de algo que não está lá. Aceitamos isso. Veja a seção sobre extensões.

## Como o PatchPilot usa a imagem

Para cada verificação, o PatchPilot sobe um contêiner novo a partir do sandbox-image, monta ou clona o código do repositório alvo, instala as dependências e roda os testes direcionados. Quando termina, o contêiner é descartado. Nada persiste dentro dele entre execuções.

O resultado volta ao processo principal, que grava o status no SQLite e atualiza o PR. O contêiner nunca escreve direto no banco. Essa separação é deliberada: o sandbox roda código de terceiros, e o banco não deve ficar ao alcance dele.

## Execução no GitHub Actions

Nos fluxos do GitHub Actions, o job usa o sandbox-image como contêiner do job ou chama o Docker para subir a imagem. Em ambos os casos a imagem é a mesma. Isso importa porque queremos que o resultado local e o resultado no CI coincidam. Quando alguém reproduz uma falha na máquina dele, deve usar o mesmo sandbox-image e não o Node instalado no sistema.

Segredos do CI não devem ser passados ao contêiner de testes, a menos que o repositório alvo exija e isso esteja explicitamente configurado. O padrão é não passar nada.

## Isolamento e segurança

Os testes executam código arbitrário vindo de dependências recém-atualizadas, o que é exatamente o cenário de risco de uma cadeia de suprimentos comprometida. O sandbox-image sozinho não é uma fronteira de segurança completa. Ele ajuda por ser mínimo, mas o isolamento real vem de como o contêiner é executado.

Regras que seguimos:

- sem acesso a credenciais do host;
- sem montar o socket do Docker dentro do contêiner;
- rede limitada ao necessário para baixar dependências, quando possível;
- usuário sem privilégios para rodar os testes, sempre que o projeto permitir;
- contêiner descartável, nunca reaproveitado.

Se uma dessas regras for relaxada para um caso específico, registre o motivo em outra nota e não aqui.

## Versionamento e atualização da imagem

A base `node:22-bookworm-slim` recebe correções do Debian e do Node ao longo do tempo. Reconstruímos o sandbox-image periodicamente para pegar essas correções. A reconstrução não deve mudar nada do que está instalado além das versões que a base traz.

Um ponto que já gerou dúvida: a tag da base é móvel. Reconstruir hoje e daqui a algumas semanas pode dar bases diferentes. Para a maioria dos casos isso é desejável, porque traz patches de segurança. Se um dia a reprodutibilidade exata virar requisito, a saída é fixar a base por digest, e isso precisa de uma decisão separada.

Subir a linha principal do Node é uma mudança de design, não uma reconstrução de rotina. Exige testar contra uma amostra de repositórios alvo antes de publicar.

## Extensões e casos especiais

Alguns repositórios precisam de ferramentas que o sandbox-image não traz, como um compilador para módulos nativos ou uma biblioteca de sistema. A resposta padrão é não adicionar à imagem comum. As opções, em ordem de preferência:

- o projeto pré-compila ou usa binários prontos;
- criamos uma imagem derivada do sandbox-image, usada só por aquele repositório;
- em último caso, ampliamos a imagem comum, se várias equipes pedirem a mesma coisa.

A imagem derivada parte do sandbox-image e acrescenta o mínimo. Assim a base comum continua pequena e a exceção fica visível e dona de si mesma.

## Armadilhas conhecidas

- Falha de teste que só aparece no sandbox-image costuma indicar dependência implícita de uma ferramenta do sistema que o projeto assumia ter. Não corrija instalando na imagem comum sem antes conversar com a equipe do projeto.
- Diferença de gerenciador de pacotes entre o que o projeto declara e o que está na imagem pode mudar a árvore de dependências instalada. Confira se o projeto fixa o gerenciador.
- Cache de instalação não sobrevive entre contêineres. Se o tempo de instalação incomoda, a solução é cache fora do contêiner, não engordar a imagem.
- Testes que escrevem fora do diretório do projeto podem falhar quando o usuário não tem permissão. Isso é sinal de teste mal comportado, não de bug da imagem.

## Decisões abertas

Ainda não decidimos se vale fixar a base por digest. Também está em aberto se o sandbox-image deve publicar uma lista legível do que contém, gerada na build, para facilitar auditoria. Ambas são boas ideias com custo de manutenção, e nenhuma é urgente.

Outra pergunta em aberto: se devemos oferecer uma variante com ferramentas de compilação como imagem oficial, em vez de deixar cada equipe criar a sua derivada. Depende de quantos pedidos aparecerem.

## Resumo para quem chega agora

O sandbox-image é pequeno por escolha. Parte de `node:22-bookworm-slim`, instala só o git e os gerenciadores de pacotes, roda como contêiner descartável e não guarda estado. Se precisar de mais coisa, crie uma imagem derivada antes de mexer na comum. Se for trocar a base ou a linha do Node, trate como mudança de design e teste contra repositórios reais primeiro.
