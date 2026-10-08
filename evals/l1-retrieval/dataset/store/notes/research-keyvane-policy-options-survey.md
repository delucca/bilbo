---
id: 01K0C3JBDY59FARXPHEWZ8XKSV
created: 2025-07-17T08:50-03:00
---

# keyvane-policy-engine: opções consideradas

Anotação rápida sobre as opções gerais que olhamos para o keyvane-policy-engine. Nada aqui está decidido; é um mapa do que existe, do que cada caminho custa e do que ainda precisa de teste. O componente responde à pergunta "esta identidade pode receber esta credencial agora?" e precisa fazer isso sem redeploy quando a regra muda.

## Contexto do problema

O keyvane-policy-engine fica entre quem pede a credencial (serviços identificados por SPIFFE, via mTLS) e quem a emite (Vault). Ele avalia regras, devolve permitir ou negar e deixa rastro do motivo. As equipes de aplicação querem escrever regras sem abrir PR no núcleo; a equipe de segurança quer revisar e auditar mudanças.

## Opção 1: usar só as políticas nativas do Vault

Mais simples: nenhuma peça nova para operar. As políticas do Vault são baseadas em caminho e capacidades, e cobrem bem o "quem lê qual segredo".

- Limite: pouca expressividade para condições contextuais (horário, origem, estado do serviço).
- Limite: a lógica de decisão fica espalhada entre o Vault e o código do Keyvane.
- Bom como camada de baixo, mesmo que haja um motor próprio em cima.

## Opção 2: motor de políticas externo e genérico

Linguagem declarativa de regras, avaliada em processo ou em sidecar. Ganha-se expressividade e testes de regra isolados. Perde-se simplicidade: mais uma linguagem para a equipe aprender e depurar.

- Dá para testar regras com tabelas de casos, fora do serviço.
- Risco de a linguagem virar um segundo lugar onde bugs de autorização se escondem.
- Precisa de um jeito claro de carregar pacotes de regras atualizados.

## Opção 3: motor próprio em Go

Regras como dados estruturados, avaliadas por código Go que a equipe já domina. Controle total sobre o formato da decisão e da auditoria.

- Menos dependências e uma única linguagem no repositório.
- Custo de manutenção é nosso: precisa de um conjunto de testes forte e de cuidado com o formato das regras.
- Tentação de crescer a linguagem de regras sem perceber; vale impor um limite de escopo desde cedo.

## Onde guardar as regras

O etcd aparece como candidato natural, por já fazer parte da pilha e por oferecer observação de mudanças (watch). A alternativa é manter as regras em repositório versionado e publicá-las no armazenamento por um pipeline.

- Fonte de verdade no repositório dá revisão, histórico e rollback por commit.
- O etcd serve como distribuição rápida, não como lugar onde se edita à mão.
- Ainda falta decidir como lidar com um nó que perdeu o watch e ficou com regras velhas.

## Identidade do chamador

A decisão depende de uma identidade confiável. A ideia geral é usar o identificador SPIFFE extraído do certificado apresentado no mTLS, e não campos enviados no corpo da requisição. As regras então casam sobre a identidade e sobre atributos derivados dela, como o domínio de confiança.

## Modelo de decisão

Pontos gerais que apareceram nas discussões:

- Negar por padrão; permitir só com regra explícita.
- A resposta deve trazer o motivo, para o auditor e para quem está depurando um acesso negado.
- Decisões precisam ser reprodutíveis: mesma entrada, mesma regra, mesma saída.
- Falha do motor ou do armazenamento deve falhar fechado, com exceção a debater para caminhos de emergência.

## Atualização de regras sem redeploy

Duas abordagens: recarga a quente a partir do armazenamento ou troca atômica de um conjunto de regras já validado. A segunda parece mais segura, porque uma regra inválida nunca chega a ser ativada. Falta avaliar o custo de validar o pacote inteiro a cada mudança.

## Auditoria e observabilidade

Cada decisão deveria gerar um registro com identidade, recurso pedido, resultado e regra responsável. Também interessam métricas de latência da avaliação e de taxa de negações, para detectar regra mal escrita antes de virar incidente.

## Perguntas em aberto

- Qual formato de regra as equipes de aplicação aceitariam escrever sem sofrimento?
- Como simular o efeito de uma mudança de regra sobre o tráfego real antes de ativá-la?
- Como separar o que o Vault já garante do que o motor precisa garantir, sem duplicar nem deixar lacuna?
- Quem aprova mudanças de regra e como isso entra no fluxo de revisão?
