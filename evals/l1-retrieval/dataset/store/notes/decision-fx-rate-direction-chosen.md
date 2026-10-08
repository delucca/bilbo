---
id: 01KQM6KECN48NHB4V0YN5YA1BC
created: 2026-05-02T08:18-03:00
---

# Direção geral do fx-rate-loader

O `fx-rate-loader` vai ficar como um serviço pequeno e com uma função só: buscar taxas de câmbio de fontes externas, guardar no PostgreSQL e deixar o restante do Ledgerlark consultar. Não decidimos nada de valor exato aqui, só a linha geral. Anotei porque a conversa se repetiu algumas vezes e não quero que alguém reabra tudo de novo.

## Contexto

A conciliação compara arquivos de liquidação das processadoras com lançamentos internos do ledger. Quando as moedas diferem, a comparação depende de uma taxa. Se cada parte do sistema buscar a sua taxa, dois lados da mesma conciliação podem usar taxas diferentes e gerar divergência falsa. Por isso a taxa precisa ter um dono único.

## Decisão geral

O `fx-rate-loader` é o único dono das taxas. Nenhum outro componente chama fonte externa de câmbio. Quem precisa de taxa lê o que o `fx-rate-loader` já gravou, e a taxa usada em uma conciliação fica registrada junto com o resultado, para dar para explicar depois.

## Armazenamento

- Taxas ficam em tabelas próprias no PostgreSQL, só com inserção; não se atualiza nem apaga linha antiga.
- Cada taxa guarda a fonte e o momento em que foi obtida.
- Correção de taxa errada entra como nova linha que substitui a anterior na leitura, sem mexer no histórico.

## Entrega para os outros serviços

Consulta síncrona por gRPC para quem precisa da taxa na hora. Quando uma taxa nova é carregada, o serviço publica um evento no Kafka, para quem quiser reagir (por exemplo, reabrir itens que dependiam de uma taxa ausente). Os dois caminhos leem da mesma base, então não devem discordar.

```text
fx-rate-loader -> PostgreSQL -> gRPC / Kafka
```

## Falhas e taxa ausente

Se uma fonte falhar, o carregador não inventa taxa nem copia a anterior em silêncio. A conciliação que depende dela marca o item como pendente de taxa e segue para revisão, em vez de passar como conciliado. Preferimos um item pendente a um erro escondido.

## Infraestrutura e o que fica em aberto

A implantação do `fx-rate-loader` é descrita em Terraform, como os demais serviços, sem configuração manual. Ficam em aberto, para notas próprias: a escolha das fontes, a frequência de carga, a tolerância entre fontes e a política de retenção. Nada disso foi decidido aqui.
