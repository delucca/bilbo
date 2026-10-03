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
- `.github/workflows/release.yml` is output of `dist generate`, from
  `dist-workspace.toml`. Never edit it by hand: the release workflow's `plan`
  job runs `dist plan` on every PR, and it fails when the file is stale.
- Use OpenSpec 1.14.0, the version in their `generatedBy` field. The
  workflows call subcommands that older releases lack.

## Conventions

- Pin every dependency exactly. Cargo pins through the committed
  `Cargo.lock`, with the caret idiom in `Cargo.toml`; ecosystems without a
  lockfile pin in the manifest. Floating ranges make builds irreproducible.

## Architecture

One crate, binary `bilbo`. Dependencies: `jiff`, `ureq` (HTTP and TLS, with
its `json` feature), `serde`, `serde_json` (the agent CLIs' JSON and the fake
embedder), `cliclack` (the wizard, only in `src/wizard.rs`), `libc` (the
wizard's guard that keeps the tty from echoing a pasted key, only in
`src/wizard.rs`) and `zeroize` (the pasted key). `Cargo.lock` is committed and
pins the build.

- `Cargo.toml`: manifest.
- `flake.nix` and `flake.lock`: the Nix flake (`distribution` spec), for
  aarch64-darwin, x86_64-linux and aarch64-linux. `packages.default` builds
  `bilbo` from `Cargo.lock` and runs the tests in its check phase. Its `src`
  is a `lib.fileset`, so a new file the build or the tests read must join it.
  It copies `plugins/bilbo/` and both marketplaces into `share/bilbo/`, which
  is a local marketplace either tool can add by path. `homeManagerModules.default`
  (`programs.bilbo`; `setup` spec) is a thin wrapper: it writes the config from
  `settings` and runs `bilbo setup --yes` on activation, passing the store root
  and home-manager's XDG folders itself, because activation reads no session
  variables. The `home-manager` input is read only by the `home-manager-module`
  check, which evaluates the module with sample settings without building
  bilbo; a consumer sets `bilbo.inputs.home-manager.follows`. `devShells.default`
  holds cargo, rustc, clippy and rustfmt from `nixpkgs` (26.05, rustc 1.95.0)
  and cargo-dist from `nixpkgs-unstable`. The check phase skips
  `every_action_is_pinned_by_sha`, because `.github` is not in `src`.
- `dist-workspace.toml`: the cargo-dist config (`distribution` spec): the
  dist version, the four targets, the shell installer, `install-path`, the
  `macos-15` runner for aarch64-apple-darwin and the
  `[dist.github-action-commits]` pins. dist also reads `Cargo.toml`'s
  `repository` and builds with its `[profile.dist]`.
- `tests/workflows.rs`: every `uses:` in `.github/workflows/` names a 40-hex
  commit SHA; local `./` actions are exempt.
- `src/main.rs`: verb dispatch, `--help`, `--version`, `Failure`, exit codes 0,
  1, 2 and the only writer of stdout and stderr (every stderr line gets
  `bilbo: `), except the wizard's prompts, which cliclack draws on stderr
  (`cli` spec).
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
- `src/command.rs`: the `Runner` trait and `System`, which runs another
  program with stdin closed and bilbo's environment, plus the PATH lookup.
  Shared by `agents` and `timer`.
- `src/agents.rs`: the `claude` and `codex` plugin commands, their JSON, the
  same-source rule (a marketplace from another source is removed before the
  add) and the plugin source: the package's `share/bilbo/`, else
  `delucca/bilbo` at `v<version>` (`setup` spec).
- `src/timer.rs`: the launchd plist and the systemd units as pure text with
  the carried store, config and XDG locations, and their load and unload
  (`setup` spec). Removal fails and keeps the files when `launchctl` or
  `systemctl` is missing.
- `src/wizard.rs`: the wizard behind the `Prompter` trait; the only user of
  cliclack. It draws on stderr without the `bilbo: ` prefix (the `cli` spec's
  exception). Its `Terminal` adapter is the only code the unit tests cannot
  reach.
- `src/setup.rs`: `bilbo setup`, plan then apply, and `--remove`
  (`setup` spec). The wizard path and `--remove` take `&mut impl Prompter`;
  `setup::tests::driven` runs them end to end with a scripted one. A config of
  only comments takes embedder flags (`config updated`, `config.bak`).
- `store`, `note`, `rank`, `config`, `embed`, `vectors`, `command`, `agents`
  and `timer` never print and never return `Failure`; they return plain values
  and `String` messages. Verbs (`setup` included) build on them, never on each
  other, return `crate::Failure` and never print.
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
- `tests/setup.rs` runs `bilbo setup` through the built binary.
  `tests/common/fakes.rs` writes fake `claude`, `codex`, `launchctl` and
  `systemctl` scripts into a temp folder that is the whole PATH, so they use
  only shell builtins. `tests/fixtures/agents/` holds recorded real outputs,
  the first line being the command; refresh them against throwaway
  `CLAUDE_CONFIG_DIR` and `CODEX_HOME` when a tool's JSON moves. The timer
  tests are per platform, and CI runs the Linux ones. The macOS suite takes
  about 11 s, because macOS scans each freshly written script; Linux takes
  under a second. The wizard's `Terminal` adapter is covered only by the
  recorded expect run in the add-setup change's `smoke.md`
  (under `openspec/changes/archive/` once archived).
- `tests/plugin.rs`: the plugin's files, skill frontmatter and versions,
  checked in CI. Locally, also run `claude plugin validate .` and
  `claude plugin validate plugins/bilbo` (one missing-version warning each is
  expected; never `--strict`) and
  `PYTHONDONTWRITEBYTECODE=1 python3 ~/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py plugins/bilbo`.
- Verification is the `Verification` line of `openspec/config.yaml`: `cargo fmt
  --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test
  --locked`. Run them as `nix develop -c <cmd>`, with `CARGO_TARGET_DIR` set to
  this checkout's `target`: a global value moves `./target/debug/bilbo`
  elsewhere.
- `.github/workflows/ci.yml`: the `verify` job runs the Verification line on
  every PR and every push to main, and the `main pull requests` ruleset requires
  it and the `nix` job. It runs Rust 1.95.0, the `rust-version` floor, and so
  does the dev shell, from the nixpkgs 26.05 pinned in `flake.lock`; a lock bump
  can bring a newer clippy that flags lints CI does not. Bump the toolchain with
  `rust-version`. The `nix` job runs `nix flake check -L` and evaluates the
  aarch64-darwin package. Actions are pinned by commit SHA.

## Releases

1. Bump `version` in `Cargo.toml` and in
   `plugins/bilbo/.codex-plugin/plugin.json` together, then run
   `nix develop -c cargo update --workspace` so `Cargo.lock` follows.
   `tests/plugin.rs` fails when the two versions differ.
2. Merge through a pull request.
3. Tag the commit on `main` that carries the bump (the tip of `main` after
   the rebase-merge) `v<version>` and push the tag. The release workflow
   publishes the GitHub Release. A tag that does not match `Cargo.toml` fails
   in `plan` and publishes nothing. Push only `v<version>` tags: the
   generated trigger also accepts `<version>` and `bilbo-v<version>`, and
   either would publish a second release.

To upgrade dist, change `cargo-dist-version` and the `nixpkgs-unstable` input
(`nix flake update nixpkgs-unstable`) together, until
`nix develop -c dist --version` matches. Then run `nix develop -c dist init
--yes` and pin every action the new `release.yml` names in
`[dist.github-action-commits]`, using the commit from
`gh api repos/<owner>/<repo>/git/ref/tags/<tag>` (dereference an annotated
tag with `gh api repos/<owner>/<repo>/git/tags/<sha>`). Run
`nix develop -c dist generate` again, and check that `cargo test --locked
--test workflows` passes.
