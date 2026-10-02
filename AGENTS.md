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

- Pin every dependency exactly. Cargo pins through the committed
  `Cargo.lock`, with the caret idiom in `Cargo.toml`; ecosystems without a
  lockfile pin in the manifest. Floating ranges make builds irreproducible.

## Architecture

One crate, binary `bilbo`; `jiff` is the only dependency. `Cargo.lock` is
committed and pins the build.

- `Cargo.toml`: manifest.
- `src/main.rs`: verb dispatch, `--help`, `Failure`, exit codes 0, 1, 2 and the
  only writer of stdout and stderr (every stderr line gets `bilbo: `) (`cli`
  spec).
- `src/store.rs`: store root resolution and listing `notes/` (`note-store`
  spec). Root resolution takes `Env` as a value, built once in `main`.
- `src/note.rs`: kinds, filename, ULID, `created`, the strict frontmatter and
  title reader, and the note renderer (`note-store` spec).
- `src/new.rs`: `bilbo new`, argument checks, atomic create (`note-create`
  spec).
- `src/check.rs`: `bilbo check`, read-only (`store-check` spec).
- `store` and `note` never print and never return `Failure`; they return
  plain values and `String` messages. Verbs build on them, never on each
  other, return `crate::Failure` and never print.
- A new verb is `src/<verb>.rs`, its `mod` line, dispatch arm and USAGE line
  in `src/main.rs`, `tests/<verb>.rs`, its own capability spec, and a MODIFIED
  `cli` spec (its Verb dispatch requirement lists the verbs).
- Unit tests live in the module they test. CLI behavior is tested in
  `tests/cli.rs`, `tests/new.rs` and `tests/check.rs` through the built binary
  with a clean environment; `tests/common/mod.rs` holds the shared runner and
  temp folders.
- Verification is the `Verification` line of `openspec/config.yaml`: `cargo fmt
  --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test
  --locked`. Run them in `nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#clippy
  nixpkgs#rustfmt -c <cmd>`, with `CARGO_TARGET_DIR` set to this checkout's
  `target`: a global value moves `./target/debug/bilbo` elsewhere.
- `.github/workflows/ci.yml`: the `verify` job runs the Verification line on
  every PR and every push to main, and the `main pull requests` ruleset
  requires it. It runs Rust 1.95.0, the `rust-version` floor, while the nix
  shell runs whatever nixpkgs ships, so a newer local clippy can flag lints CI
  does not. Bump the toolchain with `rust-version`. Actions are pinned by
  commit SHA.
