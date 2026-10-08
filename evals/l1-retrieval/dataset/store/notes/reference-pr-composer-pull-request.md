---
id: 01M3SKY5QQ0MFBBAC0WQ9833AP
created: 2026-09-30T13:56-03:00
sources:
  - "code: templates/pr-body.md.hbs"
---

# pr-composer reference

This is a quick reference for pr-composer, the part of PatchPilot that writes the pull request for a dependency upgrade. It covers where the description text comes from, what the old name was, and what to look at first when a PR body looks wrong. Kept short on purpose; related detail on grouping patch bumps lives in [[pr-composer-groups-patch]].

## What pr-composer does

pr-composer takes the result of an upgrade run and turns it into a pull request: a title, a description, labels and a branch reference. It runs after the upgrade step and after the targeted tests have been selected and executed, so it can report both what changed and what was tried.

It is written in TypeScript and runs on Node.js. In CI it is invoked from a GitHub Actions job, and in local runs from the Docker image that PatchPilot ships.

## Old name: prforge

prforge was the previous name of pr-composer. The component is called pr-composer now. If you meet `prforge` in old branches, issue titles, log lines, commit messages or someone's notes, it means the same component. New code, docs and notes should use `pr-composer` only.

## Why the rename matters when searching

Searching the repository for the old name can still turn up leftovers: comments, stale config keys, or text in old PR descriptions that were generated before the rename. Search for both `prforge` and `pr-composer` when tracing history. Do not assume a hit on the old name is a separate module.

## Where the PR description comes from

The pull request description of pr-composer is rendered from the Handlebars template at `templates/pr-body.md.hbs`. That file is the single source for the body text. There is no second template for a different PR type; variations are handled inside that one template with Handlebars conditionals.

## Template basics

The template is Markdown with Handlebars expressions. The renderer is given a plain context object built from the upgrade result. Sections that have no data are skipped with conditionals, so an upgrade with no test results does not leave an empty heading behind.

## Context passed to the template

The context carries the package names, the old and new versions, the ecosystem, the changelog excerpt when one was found, and the targeted test outcome. It also carries the repository name and the branch. Check the code that builds the context if a field is missing in the output, since the template only prints what it receives.

## Editing the description format

To change how descriptions look, edit `templates/pr-body.md.hbs` and nothing else. Do not hard-code body text in TypeScript. Keep the Markdown simple, because GitHub renders it and long tables tend to break on narrow screens.

## Escaping gotchas

Handlebars escapes HTML in double-brace expressions. Changelog text with angle brackets or ampersands can come out mangled, so triple-brace output is used only where the text is already trusted Markdown. Be careful before switching an expression to triple braces: changelog text comes from third parties.

## Grouped patch upgrades

When several patch-level bumps are grouped into one pull request, the same template is used with a list of packages instead of a single one. The grouping rules and their edge cases are in [[pr-composer-groups-patch]]. Do not duplicate them here.

## Test results section

The targeted test summary is part of the description. It states which tests were chosen and whether they passed. If the test step was skipped, the section says so instead of being dropped, so reviewers do not read silence as success.

## Labels and titles

Titles and labels are produced in code, not in the template. Only the body is templated. If a title looks wrong, look in the composer code, not in the Handlebars file.

## Storage of run data

Run records that feed the description are kept in SQLite. pr-composer reads them; it should not write back to those tables. If a field is absent from the description, check whether the run record had it before blaming the template.

## Running in Docker

The Docker image includes the templates directory, so a template edit needs an image rebuild to show up in containerized runs. Local Node runs read the file directly from the working tree.

## Running in GitHub Actions

In Actions the composer step runs after the test step and uses the same checkout. A missing template in the checkout path is a common cause of an empty body, so confirm the file exists in the job workspace.

## Common problems

- Body is empty: the template was not found, or the context was empty.
- Old name appears in text: leftover copy from before the rename, see above.
- Changelog looks garbled: escaping, see above.
- Edit has no effect in a container: the image was not rebuilt.

## Open items

Nothing is pending on this component right now. If a second template is ever needed, record the reason here before adding it, so the single-source rule stays clear.
