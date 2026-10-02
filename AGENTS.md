# bilbo

Durable memory for coding agents: notes they write and recall, and a library
of sources they cite. The product frame and vocabulary live in
`openspec/config.yaml`.

## Workflow

- Start every behavior change with an OpenSpec change: `/opsx:explore` to
  think it through, `/opsx:propose <name>` to draft it, `/opsx:apply` once
  the user approves it, `/opsx:archive` when it ships. Codex runs the same
  workflows as `$openspec-explore`, `$openspec-propose`,
  `$openspec-apply-change` and `$openspec-archive-change`.
- Treat `openspec/specs/` as the current contract and change it only through
  `/opsx:archive` or `/opsx:sync`, so every spec edit traces back to a
  reviewed change.
- Make fixes that leave behavior unchanged (typos, refactors, test-only
  edits) directly, without a change.
- A change that adds a command, a dependency manifest or a top-level
  directory also updates this file.

## Generated files

- `.claude/commands/opsx/`, `.claude/skills/openspec-*` and
  `.agents/skills/openspec-*` are output of `openspec update`. Regenerate
  them after an OpenSpec version bump instead of editing them. Keep the two
  skill folders as separate copies: each tool gets its own wording, and a
  symlink makes every `openspec update` rewrite them.
- Use OpenSpec 1.14.0, the version in their `generatedBy` field. The
  workflows call subcommands that older releases lack.

## Conventions

- Pin every dependency to an exact version in its manifest. Floating ranges
  make builds irreproducible.
