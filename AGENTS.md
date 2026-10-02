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
  `.agents/plugins/marketplace.json` is written by hand, not by
  `openspec update`.
- Use OpenSpec 1.14.0, the version in their `generatedBy` field. The
  workflows call subcommands that older releases lack.

## Conventions

- Pin every dependency exactly. Cargo pins through the committed
  `Cargo.lock`, with the caret idiom in `Cargo.toml`; ecosystems without a
  lockfile pin in the manifest. Floating ranges make builds irreproducible.

## Architecture

One crate, binary `bilbo`. Dependencies: `jiff`, `ureq` (HTTP and TLS, with
its `json` feature) and `serde`; `serde_json` is a dev-dependency for the fake
embedder. `Cargo.lock` is committed and pins the build.

- `Cargo.toml`: manifest.
- `src/main.rs`: verb dispatch, `--help`, `Failure`, exit codes 0, 1, 2 and the
  only writer of stdout and stderr (every stderr line gets `bilbo: `) (`cli`
  spec).
- `src/store.rs`: store root resolution, listing `notes/` and reading the notes
  `recall` and `index` search (`note-store` spec). Root, config and cache
  resolution take `Env` as a value, built once in `main`.
- `src/note.rs`: kinds, filename, ULID, `created`, the line splitter, the
  strict frontmatter and title reader (which also returns the valid `created`
  and the body's first line), and the note renderer (`note-store` spec).
- `src/new.rs`: `bilbo new`, argument checks, atomic create (`note-create`
  spec).
- `src/check.rs`: `bilbo check`, read-only (`store-check` spec).
- `src/rank.rs`: words (case and Latin accent folding), passages (heading
  paths, 4,000-byte parts), BM25 ranking, the embedder input and reciprocal
  rank fusion (`note-recall` spec). Shared by verbs; knows nothing of the
  store or the CLI.
- `src/recall.rs`: `bilbo recall`, read-only (`note-recall` spec).
- `src/config.rs`: config file location and the strict `key = value` reader
  (`config` spec).
- `src/embed.rs`: the embedder client: one `POST <url>/v1/embeddings` per batch
  of at most 16, a bearer token from a file or a variable, unit-normalized
  vectors. Its messages never hold the token.
- `src/vectors.rs`: the vector cache, one file per store root under the cache
  folder, keyed by FNV-1a 64 of the embedder input, replaced atomically
  (`note-index` spec).
- `src/index.rs`: `bilbo index`; writes only the cache (`note-index` spec).
  Nothing in bilbo runs it on its own: a timer, a hook or the agent does, and
  `recall` says how many passages are not indexed.
- `plugins/bilbo/`: the agent plugin (`agent-plugin` spec).
  `.claude-plugin/plugin.json` sets no `version`, so Claude Code follows
  commits; `.codex-plugin/plugin.json`'s `version` equals `Cargo.toml`'s, so
  bump them together. `skills/recall/SKILL.md` runs `bilbo recall` from PATH.
  Skill frontmatter uses only `name`, `description`, `license` and
  `allowed-tools`, the keys both tools accept.
- `.claude-plugin/marketplace.json` and `.agents/plugins/marketplace.json`: the
  Claude Code and Codex marketplaces, one `bilbo` entry each with the source
  `./plugins/bilbo`.
- `store`, `note`, `rank`, `config`, `embed` and `vectors` never print and
  never return `Failure`; they return plain values and `String` messages. Verbs
  build on them, never on each other, return `crate::Failure` and never print.
- A new verb is `src/<verb>.rs`, its `mod` line, dispatch arm and USAGE line
  in `src/main.rs`, `tests/<verb>.rs`, its own capability spec, and a MODIFIED
  `cli` spec (its Verb dispatch requirement lists the verbs).
- Unit tests live in the module they test. CLI behavior is tested in
  `tests/cli.rs`, `tests/new.rs`, `tests/check.rs`, `tests/recall.rs` and
  `tests/index.rs` through the built binary with a clean environment;
  `tests/common/mod.rs` holds the shared runner, temp folders and the fake
  embedder (a `TcpListener` on `127.0.0.1` serving vectors from a substring
  table). The `#[ignore]` speed test in `tests/recall.rs` runs with
  `cargo test --release --test recall -- --ignored`.
- `tests/plugin.rs`: the plugin's files, skill frontmatter and versions,
  checked in CI. Locally, also run `claude plugin validate .` and
  `claude plugin validate plugins/bilbo` (one missing-version warning each is
  expected; never `--strict`) and
  `PYTHONDONTWRITEBYTECODE=1 python3 ~/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py plugins/bilbo`.
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
