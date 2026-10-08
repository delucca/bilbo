---
id: 01M0PXFZ0HJK68B4MK371B6GAS
created: 2026-08-23T05:58-03:00
---

# Direção geral para o auth-gateway

Nota rápida para não ter que rediscutir isso toda semana. O `auth-gateway` é a peça que fica entre o app Android dos entregadores e o resto do backend. A direção que o time escolheu é manter essa peça fina, previsível e tolerante a falta de rede. Ela existe para dizer quem é o entregador, o que ele pode fazer e por quanto tempo isso vale. Não deve virar um lugar onde mora regra de negócio de entrega.

O texto abaixo é geral de propósito. Não há valores fechados aqui, como tempos de expiração, limites ou nomes de campos. Quando algum desses for decidido de verdade, vai para uma nota própria, com a fonte ao lado.

## Contexto

O ParcelPin é usado por motoristas de última milha. Eles trabalham em rua, dentro de prédios, em garagem e em área rural, e a conexão cai com frequência. O app captura prova de entrega (foto, assinatura, localização, observações) e precisa funcionar sem rede. Depois sincroniza quando a conexão volta.

Isso muda o que se espera de autenticação. Num app comum, se o token expirou, o usuário vai para a tela de login e pronto. Aqui, se o entregador está parado na porta de um cliente sem sinal, jogar ele para uma tela de login é o pior resultado possível. Ele perde tempo, e o cliente espera. Por isso a conversa sobre o `auth-gateway` sempre começou pelo caso offline, e só depois pelo caso feliz.

O stack é Kotlin com Android Jetpack no cliente, Firebase como base de identidade, Cloud Firestore para os dados e Protocol Buffers nos contratos entre app e backend. A direção abaixo parte disso. Não estamos trocando nenhuma dessas peças.

## O que foi escolhido

A escolha central: o `auth-gateway` é o único ponto de entrada para validar identidade e emitir permissões de curta duração para o app. O app não fala com o backend de domínio sem passar por ele, e o backend de domínio não reimplementa verificação de identidade por conta própria.

Em termos práticos, o time optou por:

- Delegar a identidade do usuário ao Firebase Authentication e não manter um cadastro de senhas nosso.
- Fazer o `auth-gateway` verificar a identidade e traduzi-la em um conjunto pequeno de permissões que o resto do sistema entende.
- Manter o estado do gateway o mais próximo possível de zero. Ele consulta, valida e responde. Se precisar guardar algo, é o mínimo para auditoria e revogação.
- Deixar a decisão de "este entregador pode ver esta rota" nas regras e nos dados do domínio, com o gateway apenas informando quem ele é e a qual operação ele pertence.

O ponto que mais gerou discussão foi o último. Houve vontade de colocar no gateway toda a lógica de autorização por rota e por pacote. A conclusão foi que isso faria o gateway crescer junto com o domínio, e cada mudança de regra de entrega exigiria deploy dele. Preferimos que ele mude pouco.

## Fluxo em alto nível

O desenho abaixo é só para situar quem lê. Não é contrato.

```text
app Android -> auth-gateway -> Firebase
                    |
                    +-> Cloud Firestore
```

O app obtém a identidade pelo Firebase, apresenta ao `auth-gateway`, que valida e devolve o que o app precisa para operar. As leituras e escritas de dados do dia a dia continuam indo para o Cloud Firestore, respeitando as permissões que o gateway ajudou a estabelecer.

O contrato entre app e gateway é descrito em Protocol Buffers. Isso foi mantido por dois motivos. O primeiro é que o app já usa esse formato para o resto da comunicação, então não entra uma segunda forma de serializar. O segundo é que o esquema fica versionável e revisável em pull request, o que ajuda quando cliente e servidor evoluem em ritmos diferentes, coisa inevitável com app instalado em celular de motorista que não atualiza na hora.

## Comportamento offline

Esta é a parte mais importante da direção, e a que mais deve ser lembrada quando alguém propuser uma mudança.

O princípio: perder a rede não pode derrubar a sessão do entregador. O app deve continuar permitindo capturar prova de entrega com a identidade que já tinha, e só exigir reautenticação quando houver rede para fazê-la sem atrito. A captura offline é o produto, então a autenticação se adapta a ela, e não o contrário.

Decisões gerais que decorrem disso:

- A credencial emitida pelo `auth-gateway` precisa durar o bastante para cobrir um turno de trabalho típico com lapsos de conexão. O tempo exato fica para outra nota, e deve ser revisado com dados de campo.
- Renovação acontece de forma oportunista, quando o app percebe que há rede, e não no meio de uma captura.
- Se a credencial vence enquanto o app está offline, as provas já capturadas ficam guardadas localmente e marcadas como pendentes. Elas são enviadas depois que a renovação funcionar. Nada é descartado por causa de sessão expirada.
- O gateway não deve rejeitar uma prova legítima só porque ela chegou tarde. A validade é avaliada considerando o momento em que a captura aconteceu, na medida em que isso puder ser confiável, e não só o momento da chegada.

Esse último ponto tem um custo: aceitar dados que chegam atrasados abre espaço para abuso, por exemplo alguém tentando fabricar uma entrega depois do fato. A mitigação escolhida é registrar bastante contexto na captura e deixar a checagem de consistência para o backend de domínio, e não endurecer o gateway a ponto de punir o uso offline legítimo.

## Renovação e revogação

Duas necessidades puxam em direções opostas. Quem opera a frota quer poder cortar o acesso de um entregador rapidamente, por exemplo quando um celular é perdido ou uma pessoa deixa a empresa. Já o entregador em campo quer que nada o interrompa.

O time aceitou uma troca explícita: a revogação não precisa ser instantânea em todos os casos, mas precisa ser eficaz em um intervalo curto e razoável, e precisa valer na próxima vez que o app falar com o gateway. Para isso:

- Credenciais emitidas pelo gateway são de vida curta em relação à sessão do Firebase. A sessão longa fica com o Firebase, e as permissões efetivas ficam com o gateway.
- Na renovação, o gateway consulta o estado atual do usuário. Se ele foi desativado, a renovação falha e o app trata isso como fim de acesso.
- O app, ao receber essa negativa, não apaga dados locais de prova ainda não enviados sem antes passar por um fluxo que deixe claro o que acontece com eles. Esse fluxo ainda precisa ser desenhado com o produto. Está como pendência abaixo.

Não há decisão fechada sobre revogação imediata por push ou por listener. Foi considerado e deixado de fora por enquanto, porque soma complexidade e depende de conexão, que justamente é o que falta no campo.

## Fronteiras e o que o gateway não faz

Para evitar deriva de escopo, ficou combinado o que está fora do `auth-gateway`:

- Não guarda dados de entrega, fotos ou assinaturas. Isso é do domínio e do Cloud Firestore.
- Não implementa regras de negócio de roteirização, atribuição de pacotes ou prazos.
- Não faz limpeza ou reconciliação de dados offline. Isso pertence ao app e à camada de sincronização.
- Não vira um proxy genérico de todas as chamadas. Se uma chamada não precisa de verificação de identidade centralizada, ela não deve passar por ele só por conveniência.
- Não expõe detalhes internos do Firebase para o app além do necessário. O app conhece o contrato do gateway, e o resto pode mudar por trás.

Sobre a última linha: houve um debate sobre deixar o app falar direto com o Firebase para tudo e usar o gateway só em operações sensíveis. A preferência final foi manter um ponto único de entrada para o que é identidade e permissão, porque isso facilita auditar, trocar de provedor no futuro se for preciso e manter o comportamento consistente entre versões do app.

## Observabilidade e erros

O gateway é onde problemas de login aparecem primeiro, então precisa ser fácil diagnosticar. A direção geral:

- Registrar o suficiente para reconstruir o que aconteceu com uma sessão sem guardar dados pessoais além do necessário. Nada de tokens completos em log.
- Distinguir, nas respostas, falha de identidade, falha de permissão e falha temporária do próprio gateway. O app precisa agir diferente em cada caso: pedir novo login, mostrar que a ação não é permitida, ou tentar de novo mais tarde sem incomodar o entregador.
- Tratar falha temporária como se fosse offline. Se o gateway estiver fora do ar, o app segue na modalidade que já usa sem rede, em vez de bloquear o trabalho.
- Acompanhar métricas de renovação que falham, para perceber cedo se uma mudança de configuração ou de versão do app está derrubando sessões em campo.

Quando for escrever mensagens de erro e códigos, isso vai para uma nota de referência, e não para esta. Aqui só fica o princípio de separar as três categorias.

## Segurança, em linhas gerais

Sem entrar em parâmetros: a comunicação entre app e gateway é sempre cifrada em trânsito; o gateway valida a origem e a integridade do que recebe antes de confiar; e o segredo que ele precisa para falar com o Firebase fica fora do código e fora do app. O app não carrega nada que permita se passar pelo gateway.

Também foi acordado que o princípio do menor privilégio vale para as permissões emitidas. Um entregador recebe o que precisa para o turno dele, e não um conjunto amplo "para garantir". Se uma função nova do app precisar de permissão a mais, isso é uma mudança revisada, não um ajuste silencioso.

Dispositivos perdidos ou comprometidos são tratados pela via da revogação descrita acima, mais o que o Android oferece para armazenar credenciais de forma protegida no aparelho. O app usa o armazenamento seguro da plataforma e não guarda credenciais em texto simples.

## Alternativas que ficaram de fora

Registro curto para não repetir a conversa.

**Autenticação própria com usuário e senha mantidos por nós.** Descartada. Aumenta a superfície de risco, exige recuperação de conta, política de senhas e tudo o que o Firebase já resolve. Não é o diferencial do produto.

**Sem gateway, app direto no Firebase e no Firestore, com regras de segurança fazendo tudo.** Considerada com seriedade, porque é mais simples. Perde em dois pontos: fica difícil centralizar auditoria e revogação, e as regras do Firestore acabam carregando lógica que fica espalhada e difícil de testar. Pode ser que parte do controle continue nas regras do Firestore, mas a emissão de permissões fica no gateway.

**Gateway com toda a autorização por recurso.** Descartada pelo motivo já dito: acopla o gateway ao domínio.

**Sessão que exige rede a cada operação sensível.** Descartada por quebrar o uso offline, que é a razão de existir do produto.

## Riscos que reconhecemos

- Credencial longa demais aumenta a janela de uso indevido; curta demais prejudica o campo. Vamos ajustar com observação real, não por palpite.
- Aceitar provas atrasadas depende de a checagem de consistência do domínio ser boa. Se ela for fraca, a decisão do gateway de não punir atraso vira brecha.
- Dependência forte do Firebase para identidade. Aceitamos isso, mas mantendo o contrato do gateway independente de detalhes do provedor para não nos prender mais do que o necessário.
- Versões antigas do app em campo. O contrato em Protocol Buffers precisa evoluir de forma compatível, e o gateway deve tolerar clientes mais velhos por um período.

## Pendências

- Definir com o produto o que acontece com provas pendentes quando o acesso do entregador é revogado de vez.
- Escrever uma nota separada com os valores operacionais (duração de credenciais, janelas de renovação, limites), quando houver dados que os sustentem.
- Escrever uma nota de referência com as categorias de resposta de erro do `auth-gateway` e como o app reage a cada uma.
- Revisar, depois de algum tempo em uso real, se a divisão entre gateway e regras do Firestore está no lugar certo.

Se alguém for mexer no `auth-gateway` e a mudança contradizer algo acima, vale parar e atualizar esta nota antes, ou junto, para ela não ficar mentindo.
