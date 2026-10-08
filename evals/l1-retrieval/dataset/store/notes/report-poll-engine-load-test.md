---
id: 01KVPK2HSMZ90R504HR3C48QH2
created: 2026-06-21T23:38-03:00
---

# poll-engine load test report: throughput and latency on a single node

A load test on an 8 vCPU node showed poll-engine sustaining 18000 votes per second with a p99 latency of 140 ms. This note records that result, what it does and does not tell us, and what to check before anyone quotes it in a plan or to a customer. It is written quickly from the test outcome; the setup details that were not captured are called out as gaps instead of guessed.

## Headline result

On one node with 8 vCPU, poll-engine held 18000 votes per second for the duration of the run, and the p99 latency stayed at 140 ms. Both figures belong together. The throughput number alone says nothing about how the tail behaved, and the latency number alone says nothing about the load it was measured under. When quoting, always give both, and always say it was a single 8 vCPU node.

## What the numbers mean

Throughput here is accepted votes per second, counted at the engine. Latency is the time from a vote arriving at the engine to the point where the engine considers it handled, taken at the 99th percentile. So one vote in a hundred took longer than 140 ms. Median and lower percentiles were better than that, but we do not have those figures in this note, so do not infer them.

## Why this test was run

Large virtual events produce bursts. A producer opens a poll on stage, the host says "vote now", and a big share of the audience taps within a short window. The engine has to take that burst without dropping votes and without letting the live results lag so far behind that the host reads stale numbers on air. We wanted a single-node ceiling so capacity planning could start from a measured figure instead of a hope.

## Test shape

The test drove votes at the engine from load generators outside the node under test. The node had 8 vCPU. Votes went through the same path real clients use, meaning the WebSocket entry on the Phoenix side and then the poll-engine processing. The database behind it was CockroachDB. The Next.js front end was not part of the load; it was not in the path being measured.

## What was held constant

One node, one engine instance, one set of polls active at once during the run. Hardware class stayed the same for the whole test. The vote payloads were the normal small ones. Nothing else heavy was scheduled on the node while the test ran. If any of that changes in a later run, the comparison with this result is weaker and should say so.

## How the engine handles a vote

A vote arrives over a WebSocket connection handled by Phoenix, gets validated against the poll state, is counted, and is queued for durable storage. Counting in memory is what lets the live tally stay fast; storage is what makes the result survive a restart. The result above is about the whole path together, not about the in-memory counter by itself. A microbenchmark of just the counter would give a much larger number and would be misleading as a capacity figure.

## Where the time goes

We did not profile in detail during this run, so this is a reading and not a measurement. The tail latency is most likely driven by queueing when the storage writes fall briefly behind and by scheduler contention when all cores are busy. The Erlang VM spreads work across schedulers well, but at saturation any extra work shows up as waiting. Treat this as a hypothesis to test, not a finding.

## Role of CockroachDB

Votes are written to CockroachDB, which is distributed and pays for consistency with extra round trips. The engine batches writes so that each vote does not cost a separate round trip. The batching is the main reason the single-node figure is as high as it is. It is also the main place a regression would hide: a change to batch handling can move throughput or the tail without touching anything in the engine code that people usually review.

## Role of WebSockets

Each connected viewer holds a socket. Vote traffic rides on those sockets, and so does the pushing of live results back out. This test focused on the inbound vote side. Fan-out of result updates to a very large audience is a separate load and was not what this figure covers. Do not read this number as the limit of the whole live experience.

## Limits of the result

The result is for a single node and a single run profile. It does not show how the engine scales across several nodes, and it does not prove the number holds for hours. It also does not cover the case where moderation actions run at the same time as heavy voting. Real events mix voting, Q&A and moderation, and the mix could cost more than a pure vote stream does.

## Gaps in what we recorded

We did not keep the exact build of the engine, the exact CockroachDB topology, or the full latency histogram. We also did not record how many distinct polls were active or how votes were spread across them. If someone needs to reproduce the result, those are the first things to pin down and write into a new note. Until then the figure is a good guide and not a guarantee.

## How to use the figure for planning

Use 18000 votes per second as the rough ceiling for one 8 vCPU node at a p99 of 140 ms, and leave headroom. Do not plan to run a node at its ceiling during a live event. A sensible habit is to size so the expected peak sits well below the measured ceiling, because audience bursts are hard to predict and the tail gets worse as the node nears saturation. Pick the headroom with the producers, who know how spiky their events are.

## Risks if the number is misquoted

The risk we care about is a sales or planning conversation that takes the figure as per-event or per-cluster capacity. It is neither. It is one node. A second risk is dropping the latency half and promising throughput without the tail. A third is comparing it with numbers from other systems that count different things, such as connections held instead of votes handled.

## Follow-up tests worth running

Repeat the run with moderation traffic mixed in. Repeat with result fan-out to a large number of connected viewers at the same time. Run a long soak to see whether the tail drifts. Run on a different node size to see whether throughput tracks the core count or flattens. Capture the full latency distribution each time, not only the p99, and keep the engine build and database layout with the results.

## Open questions

Does the ceiling come from CPU, from the storage path, or from the socket layer? Would more batching raise throughput at the cost of a worse tail, and is that trade acceptable on stage? How does the engine behave past the ceiling: does it shed load cleanly, or does the tail blow up? Nobody has answered these yet, and the answers decide how much headroom we need.

## Who to tell

Event producers and community managers care about the live feel, not the numbers, but the people who plan capacity and talk to large customers need the headline and its limits. Pass them the sentence from the top of this note unchanged, including the node size and the p99. If a later run changes the figure, update this note instead of writing a second one on the same subject.

## Status

The result stands as measured on that one run. It is a baseline for poll-engine, not a target and not a promise. Next step is the mixed-load run with moderation, then the fan-out run, each recorded with full setup so the baseline can be compared fairly.
