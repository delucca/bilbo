---
id: 01KMMYA6WKH3Y3DF2GH58RK5VX
created: 2026-03-26T08:26-03:00
---

# ledgerlark-infra pins Terraform to 1.11.2

This note replaces the earlier note "ledgerlark infra pins terraform". The new value is `1.11.2`: ledgerlark-infra pins Terraform to version `1.11.2`, after the provider lock file was regenerated. That pin replaces the earlier one. Anything that still names the old version is stale and should be fixed or ignored.

`llinfra` is short for `ledgerlark-infra`. You will see both in chat, branch names, and CI job labels. They mean the same component. In this note I use the full name `ledgerlark-infra` except where the short form is the point.

```
component: ledgerlark-infra
short name: llinfra
terraform pin: 1.11.2
```

## Decision

Terraform for `ledgerlark-infra` is pinned to `1.11.2`. Everyone who plans or applies against this component uses that version: engineers on laptops, coding agents, and CI. The provider lock file was regenerated at the same time, so the pin and the lock file now describe the same toolchain. They should be changed together from now on, never one without the other.

The old pin is gone. I am not recording its value here on purpose. Someone searching for it should land on this note and read the new value, not copy the old one by accident.

## Why the pin changed

The earlier pin had drifted from what people actually ran. Some machines had a newer Terraform than the pin allowed, some had an older one, and the provider lock file had been produced under whichever version the last person happened to have. That is the usual way a lock file ends up out of step with the declared version.

Regenerating the lock file under one known version removes the guessing. After that, the pin could be set to the version that produced the lock file, and that version is `1.11.2`.

## What was actually done

The provider lock file for `ledgerlark-infra` was regenerated from scratch under `1.11.2`. The Terraform version constraint in the configuration was then set to match. The plan output afterward was read through to confirm it showed no unexpected changes to existing resources.

I did not change any provider versions on purpose as part of this. If a provider version moved because of the regeneration, that is a side effect of re-resolving constraints, and it should show up in the lock file diff in review. Look at that diff before approving anything.

## Scope of the pin

The pin covers the whole `ledgerlark-infra` component. That includes the Terraform that describes the PostgreSQL databases, the Kafka topics and their access settings, and the network and service pieces that the Go services and the gRPC endpoints sit on. It is one pin for the component, not one per module.

It does not cover the Go toolchain used by the Ledgerlark services, and it does not cover client library versions for PostgreSQL, Kafka, or gRPC. Those are separate decisions in separate places.

## How to check your local version

Run the Terraform version command and compare the first line to `1.11.2`. If it differs, install the right version before doing anything else in `ledgerlark-infra`. Do not plan "just to look" with the wrong version, because a plan can still write state-related metadata or fail in confusing ways when the versions do not match.

If you use a version manager, make sure the project picks the pinned version automatically. If it does not, fix your manager configuration, not the pin.

## Rules for working in ledgerlark-infra

- Use `1.11.2` for every plan and every apply.
- Do not commit a lock file produced under another version.
- Do not edit the lock file by hand. Regenerate it with the tool.
- If a plan shows changes you did not make, stop and find out why before applying.
- A change to the pin is its own change, with its own review, and it does not ride along with resource changes.

## Changing the pin later

When it is time to move to a newer Terraform, treat it as a small project. Pick the target version, regenerate the provider lock file under it, run a plan against every environment, and read the plan. Only then change the pin. Then write a new note that replaces this one, the way this note replaces the earlier one, so there is always exactly one note that holds the current value.

Do not bump the pin because a newer version exists. Bump it when there is a reason, such as a fix we need or a provider that requires it.

## Consequences for CI

CI for `ledgerlark-infra` must run the same version as the pin. If a CI image or a setup step names a version, it has to say `1.11.2`. A mismatch between CI and the pin is a bug in CI, not a reason to loosen the pin.

A pipeline that installs "latest" is the failure mode this decision exists to prevent. If you find one, change it to the pinned version.

## Consequences for coding agents

An agent working in `ledgerlark-infra` should read the pin before running any Terraform command, and should refuse to run if the installed version differs. It should not try to work around that by editing the pin or the lock file to match what is installed. The right fix is to install the pinned version.

An agent should also not regenerate the lock file unless the task is specifically about the pin or about provider versions. Regeneration produces a large diff that is hard to review when mixed with other work.

## Consequences for review

Reviewers of a change in `ledgerlark-infra` should look for three things about the pin. First, the version constraint still says `1.11.2`. Second, the lock file was not touched unless the change is about providers. Third, nothing in the change quietly depends on a newer Terraform feature.

If a change needs a newer feature, that is an argument for changing the pin in a separate change, not for sneaking the dependency in.

## Alternatives considered

One option was a loose constraint, allowing any compatible version in a range. That was rejected because the lock file only protects providers, not Terraform itself, and a range lets different people produce different plans from the same code.

Another option was to leave the pin as it was and just regenerate the lock file. That was rejected because the old pin was the source of the drift, and regenerating the lock file under a version the pin did not allow would have left them disagreeing.

A third option was to pin per module. That was rejected as too much bookkeeping for a component this size.

## Risks

The main risk is that someone has an old version on their machine and does not notice. The version check above is the guard. A second risk is that the regenerated lock file hides a provider change inside a large diff. Reviewers should read that diff, not skim it.

A third risk is that the pin goes stale because nobody revisits it. That is acceptable for a while. Stale and consistent is better than fresh and inconsistent, as long as the pin still works for the providers we use.

## What this does not decide

This note does not decide how state is stored, how environments are split, or how secrets reach the infrastructure. It does not decide anything about Ledgerlark's reconciliation logic, which compares card-processor settlement files with internal ledger entries and flags mismatches for review. It only fixes which Terraform builds the infrastructure under that logic.

## Open questions

- Whether the version manager configuration should live in the repository so that the right version is picked automatically for everyone.
- Whether CI should fail loudly, with a clear message, when the installed version differs from the pin.
- Whether a scheduled reminder to review the pin is worth the noise.

None of these block the current decision. They are improvements that make the pin harder to get wrong.

## Naming

The component is `ledgerlark-infra`. People shorten it to `llinfra`. Use the full name in notes, commits, and review titles so that searches find it. The short form is fine in chat. If you write a new note about this component, name it `ledgerlark-infra` in the text and mention `llinfra` once so that a search for either word finds it.

## Quick answers

What version does ledgerlark-infra pin Terraform to? `1.11.2`.

What does llinfra mean? It is short for `ledgerlark-infra`.

Why did the pin change? The provider lock file was regenerated, and the pin was set to the version that produced it.

What happened to the earlier pin? It was replaced. This note supersedes the earlier note about the Terraform pin.

Who must follow the pin? Engineers, coding agents, and CI, for every plan and every apply in `ledgerlark-infra`.

## Follow-ups

Check that CI names `1.11.2` wherever it names a version at all. Check that nobody has a local lock file that differs from the committed one. If either check finds a problem, fix the environment to match the pin and leave the pin alone.

If the pin has to change, write a replacement note and say in one sentence that it replaces this one, as this note did.
