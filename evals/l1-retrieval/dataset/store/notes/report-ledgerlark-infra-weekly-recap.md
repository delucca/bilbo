---
id: 01KHSR9HJRBMB3MP09AMF0KACN
created: 2026-02-18T22:30-03:00
---

# ledgerlark-infra weekly recap

This week on ledgerlark-infra was mostly cleanup and tightening, not new surface area. Most of the time went into the Terraform layout, the Kafka side of the settlement intake, and getting the database environments to look more alike. Nothing here is finished in a way I would call done, so read it as where things stand, written quickly so the next session does not have to rebuild the picture.

The component covers what Ledgerlark needs to run: the data store, the message bus, the service-to-service plumbing and the Terraform that ties them together. The stack for reference:

```
PostgreSQL
Go
Apache Kafka
gRPC
Terraform
```

## What moved

The biggest chunk of work was the Terraform modules. They had grown by copy and paste, and each environment carried slightly different versions of the same blocks. I pulled the shared parts into fewer modules and made the environment definitions thinner. The goal is that a change to how a topic or a database is declared happens in one place and shows up everywhere on the next plan.

The second chunk was Kafka. The settlement intake path depends on topics being configured the way the consumers expect, and a few of those settings lived only in someone's memory or in a console. I moved them into code so they are reviewable. This is the kind of drift that bites later, when a replacement environment comes up and behaves differently from the old one.

The third chunk was PostgreSQL. I looked at how the ledger-side and the reconciliation-side schemas are provisioned and made the provisioning steps more repeatable. No schema redesign. Just the infrastructure around it: roles, extensions, parameter groups and the places where those were set by hand.

Smaller items: some tidying of variable names so they say what they hold, removal of a few unused resources that were left from earlier experiments, and a pass over the outputs so downstream consumers get what they need without reading state by hand.

## Terraform layout

The module split now follows the way people think about the system rather than the way the cloud provider groups things. Data stores sit together, messaging sits together, and the networking and access pieces sit apart so they can be reviewed by whoever owns that concern.

Things I noticed while doing it:

- Several defaults were silently relied on. When I made them explicit, a couple of environments turned out to be using different ones than I assumed.
- Some resources were named in ways that would force a recreate if I renamed them. I left those names alone and added a note in the code instead of forcing a destroy and create.
- The state is split reasonably, but there is still one place where two stacks read each other's outputs in a loop-like way. It works. I do not like it. It is on the list.

I did not run anything against a production-like environment this week without a plan review first. All plan output was read before applying. Where a plan showed a replacement I did not expect, I stopped and traced it before going on.

## Kafka and the intake path

Settlement files get turned into events, and the reconciliation side consumes them. The infrastructure part is the topics, their retention and partitioning behavior, the access rules for producers and consumers, and the dead-letter handling for records that cannot be parsed.

What changed:

- Topic declarations are now in code, with the settings that matter for ordering and replay written down rather than inherited.
- Access rules were narrowed so each service touches only what it needs. This surfaced one consumer that had broader access than its job called for.
- The dead-letter path got a clearer owner. Before, nobody was sure who was supposed to look at it. Finance operations care about unmatched records, so a stuck parse failure looks to them like a missing settlement, and that is a bad way to find out.

Open concern: replay. If a consumer has to reprocess a stretch of settlement events, the reconciliation logic has to be safe to run twice. That is mostly an application matter in the Go services, but the infrastructure has to keep enough history for replay to be possible. I want to confirm the retention settings match what the reconciliation team expects, and I have not done that yet.

## PostgreSQL provisioning

The database work was about making creation repeatable. The reconciliation job writes mismatch flags that reviewers read, so the store has to be dependable and its settings have to be the same across environments, or testing means little.

- Roles and grants are now declared rather than applied by hand. The read-only role used by reviewers is separate from the one the services write with.
- Backup and recovery settings were checked against what is declared. They match in most places. One environment had a setting that differed and I brought it in line.
- Extensions and parameter settings are listed in code. A few were only set in the live instance and are now recorded.

Not done: a real restore drill. Backups existing is not the same as restores working, and I would like someone to rehearse it and write down how long it takes and what breaks. That belongs in its own note once it happens.

## gRPC and service connectivity

Less change here. The services talk over gRPC, and the infra side owns the load balancing, certificates and the rules for which service can call which. I reviewed the rules and removed a few that no longer matched any live caller.

Two things to keep in mind. First, certificate rotation is still partly manual, and it will be the first thing to hurt when someone is away. Second, health checks for the gRPC services should reflect whether the service can actually reach its database and its topics, not just whether the process is up. I found at least one service where the check is shallower than it should be. I have not changed it because it needs the service owner to agree on what counts as healthy.

## Problems and gotchas

- Renaming Terraform resources looks harmless and is not. It can trigger replacement of something stateful. Use moved blocks or state moves, and read the plan.
- Environment differences hide in defaults. When two environments behave differently, compare the resolved values, not only the files.
- Kafka settings changed in a console will be reverted by the next apply, or worse, will not be noticed at all. Treat the code as the only source.
- The dead-letter topic fills quietly. Without someone watching, it just grows.
- Cleaning up unused resources is only safe after checking that nothing reads them. Two I thought were unused were referenced by a scheduled job.

## Next up

Roughly in priority order:

1. Confirm Kafka retention against replay needs with the people who own reconciliation logic.
2. Break the circular read between the two stacks.
3. Rehearse a database restore and write up the result separately.
4. Automate certificate rotation, or at least make it a documented routine with an owner.
5. Agree on deeper health checks for the gRPC services.
6. Put an alert on the dead-letter path so a parse failure reaches a person before finance operations notices a gap.

## Questions for others

- Who owns the dead-letter topic going forward? I proposed the reconciliation team, but nobody has confirmed.
- Does finance operations have any expectation about how far back a settlement can be replayed? If so, retention should be set from that, not from guesswork.
- Is anyone still using the older experimental resources I left in place? I kept them out of caution and would like to delete them once someone says no.

That is the state of ledgerlark-infra. The structure is better than it was, the unwritten settings are mostly written, and the remaining risks are about recovery, rotation and ownership, not about the layout.
