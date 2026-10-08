---
id: 01K5DZG7G452XNXJKF2GPPFDB5
created: 2025-09-18T05:36-03:00
---

# pr-composer: template rendering and body assembly tooling

We settled how pr-composer builds pull request text. It uses a small, plain template layer in TypeScript with the data fed in as typed objects, and it does not use a heavy templating engine or an LLM to write descriptions. Reasons below, mostly so nobody reopens this without new facts.

## What we chose

pr-composer assembles the PR title and body from structured inputs: the upgrade plan, the test selection result, and the run status. Each section of the body is a function that takes a typed object and returns a string. The sections are joined in a fixed order. Markdown is written by hand inside those functions, with a few shared helpers for tables, collapsible blocks and links.

Anything that has to survive between runs, such as the last body we posted for a given upgrade, goes into SQLite. Posting itself goes through the GitHub API client the rest of the project already uses, called from a GitHub Actions job inside the Docker image.

## Why not a templating engine

We looked at the usual options. A string-template library would have given us files that non-engineers could edit, but nobody asked for that. Platform engineers who use PatchPilot want predictable output, and they change behavior through config, not by editing templates.

The bigger problem was typing. With templates in separate files, a renamed field breaks at render time, in a PR, in someone else's repo. With functions over typed objects the compiler catches it before anything ships.

## Why not generated descriptions

Generated prose was considered for the summary of what changed in a dependency. Rejected for now. The output varies from run to run, which makes diffing the body of an updated PR noisy, and reviewers lose trust fast when one sentence is wrong. Changelog links and the raw facts we already have are enough. If we revisit this, it should be an optional section that is clearly labeled and can be turned off.

## Consequences and gotchas

- Updating an existing PR means regenerating the whole body and comparing it with the stored one. Stable section order and no timestamps inside the body keep that comparison meaningful. Do not add anything volatile to the body text.
- Long test output must be truncated by a helper before it goes in, since the platform limits body size. Put the full output in a check run or a comment instead.
- Helpers escape user-controlled strings such as package names and branch names. Keep using them and do not interpolate raw strings into the Markdown.
- Snapshot tests cover each section function. When a section changes on purpose, update the snapshot in the same change and look at the diff by eye.

## Open points

- Whether teams should be able to append their own fixed text to the body. Likely yes, as a config field rendered last, not as a template override.
- Whether the collapsible blocks render well enough in all the places people read PRs, such as mobile and email notifications. Not checked yet.
- If section count grows a lot, revisit the split into functions, but keep the typed-input rule.
