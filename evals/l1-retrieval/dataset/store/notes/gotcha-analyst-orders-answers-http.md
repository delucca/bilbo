---
id: 01KS5PJ5ANTB0NZ7WPNQVB63BD
created: 2026-05-21T13:40-03:00
---

# Gotcha: limite de taxa e HTTP 429 no analyst-orders-api

O analyst-orders-api tem um limite de taxa por chamador, e ele aparece de um jeito que engana: quando o limite estoura, a API responde `HTTP 429 Too Many Requests`, e o limite é de `60 requests per minute`. Quem não sabe disso costuma achar que o serviço caiu, que o Snowflake está lento ou que o job do Airflow quebrou. Não é nada disso. É só o limite. Esta nota registra o comportamento, como reconhecer e o que fazer.

## O que acontece

O analyst-orders-api atende os analistas de merchandising que consultam, revisam e disparam pedidos de reposição gerados pelo ShelfSense. Cada chamador tem uma cota. Passou de `60 requests per minute`, a resposta vira `HTTP 429 Too Many Requests` até a janela seguinte liberar espaço.

Pontos que importam:

- O limite vale por chamador, não pelo serviço inteiro. Um script abusivo de um analista não deveria travar os outros, mas trava o próprio analista.
- A resposta 429 não significa que o pedido foi processado pela metade. A requisição rejeitada não gera nem altera pedido nenhum.
- O erro é de cliente, não de servidor. Não adianta reiniciar o analyst-orders-api nem mexer em Spark ou Delta Lake para resolver.

## Como reconhecer

Sintomas típicos:

- Uma planilha, notebook ou script que faz um laço de chamadas começa bem e depois passa a falhar quase todas as chamadas seguidas.
- O log do cliente mostra `HTTP 429 Too Many Requests` em sequência, sem nenhum outro erro misturado.
- Rodar uma única chamada à mão, logo depois, funciona normalmente.
- A falha some sozinha depois de um tempo, sem ninguém ter mexido em nada.

Esse último ponto é a pista mais forte. Erro que some sozinho em cerca de um minuto, com o serviço saudável, quase sempre é o limite de taxa.

## Onde costuma pegar

Os casos mais comuns, pela ordem de frequência:

1. Scripts de analistas que consultam pedido por pedido em vez de buscar em lote.
2. Tarefas do Airflow que fazem fan-out de muitas chamadas em paralelo, uma por loja ou por categoria. Cada tarefa paralela conta para o mesmo chamador se usarem a mesma credencial.
3. Retentativas automáticas sem espera. O cliente leva um 429, tenta de novo na hora, leva outro 429 e piora o próprio problema.
4. Várias pessoas ou processos compartilhando uma credencial de serviço, somando a cota de todos num chamador só.

O caso 4 é o mais traiçoeiro: cada pessoa faz poucas chamadas e mesmo assim o limite estoura.

## O que fazer

Do lado do cliente:

- Espalhar as chamadas ao longo do tempo e ficar abaixo de `60 requests per minute` com folga, sem tentar andar rente ao teto.
- Tratar `HTTP 429 Too Many Requests` como sinal para esperar, não como falha final. Esperar e tentar de novo, com intervalo crescente entre as tentativas.
- Agrupar consultas quando possível, para gastar menos chamadas por tarefa.
- Em tarefas do Airflow, limitar a concorrência da tarefa que chama a API, para que o conjunto de execuções paralelas não passe da cota.
- Não compartilhar credencial entre processos que rodam ao mesmo tempo, se a ideia for ter cota separada.

Exemplo de como o erro aparece para o cliente:

```
HTTP 429 Too Many Requests
```

Não tem código para copiar aqui de propósito: a espera e o intervalo dependem do cliente de cada um. O que vale é o princípio de recuar ao receber essa resposta.

## O que não fazer

- Não abrir incidente de indisponibilidade do analyst-orders-api só por causa de 429. Primeiro conferir se o chamador passou de `60 requests per minute`.
- Não subir paralelismo para "ganhar tempo" quando a API começa a recusar. Isso só aumenta a taxa de recusa.
- Não mascarar o erro com retentativa imediata e infinita. Gera tráfego inútil e mantém o chamador sempre acima do limite.
- Não assumir que dados de reposição estão errados porque a chamada falhou. A recusa é anterior a qualquer leitura ou escrita.

## Dúvidas em aberto

Não foi confirmado nesta sessão se a janela é deslizante ou fixa, nem se a resposta traz algum cabeçalho dizendo quanto esperar. Vale checar no código do serviço antes de desenhar uma espera precisa no cliente. Também não ficou registrado se existe forma de pedir cota maior para um chamador específico; se alguém descobrir, atualizar esta nota em vez de criar outra.
