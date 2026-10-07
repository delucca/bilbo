# bilbo

Durable memory for coding agents: notes they write and recall, and a library
of sources they cite. Product frame, vocabulary and stack live in
`openspec/config.yaml`; install and setup in `docs/`. Setup, tests and
releases for humans: `CONTRIBUTING.md`.

## Commands

Run everything in the dev shell. A global `CARGO_TARGET_DIR` sends
`./target/debug/bilbo` elsewhere: point it here first.

```sh
export CARGO_TARGET_DIR="$PWD/target"   # only when a global CARGO_TARGET_DIR is set
# Verification (the `Verification` line of openspec/config.yaml; CI's verify job)
nix develop -c cargo fmt --check
nix develop -c cargo clippy --locked --all-targets -- -D warnings
nix develop -c cargo test --locked
# The package and its tests in the Nix sandbox, and the module checks (CI's nix job)
nix flake check -L
# Speed tests, ignored by default
nix develop -c cargo test --release --test recall --test digest -- --ignored
# After touching plugins/ (one missing-version warning each is expected; never --strict); Codex's needs Codex installed
claude plugin validate . && claude plugin validate plugins/bilbo
PYTHONDONTWRITEBYTECODE=1 python3 ~/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py plugins/bilbo
```

## Workflow

- Start every behavior change with an OpenSpec change: `/opsx:explore` to
  think it through, `/opsx:propose <name>` to draft it, `/opsx:apply` once
  the user approves it, `/opsx:archive` when it ships. Codex:
  `$openspec-explore`, `$openspec-propose`, `$openspec-apply-change`,
  `$openspec-archive-change`.
- `openspec/specs/` changes only through `/opsx:archive` or `/opsx:sync`.
  Git ignores the archive folder; proposals, designs and specs name no host,
  person or other change. Fixes that leave behavior unchanged need no change.
- Update this file when a change adds a command, a generated file or a rule
  that the code, the specs and the docs cannot show. A rule needs to be
  non-obvious, hit more than once and actionable; module descriptions belong
  in specs and `//!` docs. Delete a rule in the change that makes a test
  enforce it.

## Architecture rules

`tests/layout.rs` enforces the shape of `src/` (domains, verbs, Shared
Kernel, `mod.rs`, `pub` only, no re-exports, no cycles, crate placement); its
failures say what to change. Beyond it:

- `src/main.rs` writes all stdout and stderr and prefixes every stderr line
  with `bilbo: `. The one exception is `Terminal` in `src/host/prompt.rs`,
  which cliclack draws on stderr without the prefix.
- `digest` returns a `digest::Outcome` and `main` always exits 0 for it: a
  prompt hook that exits 2 blocks the prompt.
- A non-verb file of `note` or `identity` that uses `sync` closes a cycle
  (the check skips verb files): call `sync` from the verb.
- A new crate edits `PLACEMENT` in `tests/layout.rs` and is justified in the
  change's `design.md`. A new verb also needs its dispatch arm, its `OVERVIEW`
  line and `PAGES` entry in `src/main.rs`, a `HELP` page beside its parser,
  its `VERBS` and `PARSERS` entries in `tests/cli.rs`, a `## <verb>` section
  in `docs/reference/commands.md`, `tests/<verb>.rs`, its own capability spec
  and a MODIFIED `cli` spec (its Verb dispatch requirement lists the verbs).
- A new `library` subcommand adds its word to `library::corpus::RESERVED`, or
  a corpus of that name shadows it.
- Tests stay offline: a test that gets past planning with `--embedder-local`
  calls `place_model` first; only the `#[ignore]`
  `host::model::tests::pinned_url_honors_a_range` reaches Hugging Face.
- `identity/script.rs`, the scripted `Prompter`, is test-only; no test checks
  it.
- Callers keep module-qualified calls: `use crate::shared::store;`, then
  `store::root(..)`.

## Generated files

- `.claude/commands/opsx/`, `.claude/skills/openspec-*` and
  `.agents/skills/openspec-*` come from `openspec update` at OpenSpec 1.14.0
  (older releases lack subcommands the workflows call). Regenerate, never
  edit. Keep the two skill folders as separate copies: a symlink makes every
  `openspec update` rewrite them.
- `.agents/plugins/marketplace.json` is written by hand. `release.yml` comes
  from `dist generate` over `dist-workspace.toml`: edit that and regenerate, or
  the release `plan` job fails.
- The GitHub wiki is generated from `docs/` by `wiki.yml` and
  `.github/wiki.py`: never edit it by hand.

## Gotchas

- `land` and `check` treat any hidden entry in `<root>/library/` as absent;
  the `land` lock, `<root>/library/.lock`, is one: never list or remove it.
- `flake.nix` builds from a `lib.fileset`: a new file the build or the tests
  read must join it, or `nix flake check` fails while cargo passes.
- CI's verify job runs the tests with the version bumped to 99.99.99, and
  its nix job at the real version. Read the version from
  `env!("CARGO_PKG_VERSION")` in tests, never a literal.
- Bump the Rust pin in `ci.yml` (1.95.0) with `rust-version` in `Cargo.toml`.
  A `flake.lock` bump moves the dev shell's toolchain and can bring new lints.
- CI runs on Linux: it skips the macOS-only tests in `src/host/timer.rs`,
  `src/setup/` and `tests/setup.rs`, and runs only the Linux branch of
  `src/host/swap.rs`. Run those on macOS after touching them, and run
  `docs/manual-tests.md` after touching `Terminal` in `src/host/prompt.rs`,
  which the unit tests cannot reach.
- `tests/<verb>.rs` runs the built binary in a clean environment with the
  fakes in `tests/common/`, never a real `claude`, `codex`, `launchctl`,
  `systemctl` or `llama-server`.
- Tests that run `bilbo watch` poll with a deadline, never a fixed sleep. A
  check that nothing happened waits on a barrier: a later edit to another
  note that it polls for.
- On macOS a child inherits a pipe another thread is creating. A test that
  runs `bilbo` from a second thread drops its `Watcher` before joining that
  thread, or the join waits forever
  (`history_does_not_block_a_starting_watcher`).
- Test crash points and races through the step hooks, never by killing a child.
- `config::is_local` calls `0.0.0.0` remote, yet a connect to it reaches
  `common::Fake`: a test that needs a remote embedder uses
  `http://0.0.0.0:<fake.port()>`.
- Binary tests have no terminal: the ceremony and the pairing exchange are
  tested in one process, with the scripted prompter and `pair::run`'s
  injected terminal, never through a hook.
- Fixtures under `tests/fixtures/device/`, `agents/` and `pages/` are
  recordings, never hand-edited (`CONTRIBUTING.md#fixtures`). Unit tests read
  them through `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/...")`.
- A skill's frontmatter uses only `name`, `description`, `license` and
  `allowed-tools`, the keys both Claude Code and Codex accept; `tests/plugin.rs`
  checks the existing skills by name, not a new one. A heredoc body is data:
  the check skips from a line ending in `<<'EOF'` to the line `EOF`. Use that
  exact delimiter.
- Every command in `plugins/bilbo/hooks/hooks.json` must never pass on
  bilbo's exit code (`; exit 0`: exit 2 blocks the prompt) and must stay
  quiet without `bilbo` on `PATH`.
- PreCompact and PostCompact output never reaches the model in Claude Code or
  Codex; SessionStart with the matcher `compact` does.
- Codex skips an untrusted plugin hook in silence. `bilbo setup` trusts it
  through `codex app-server`, never by editing `config.toml`; changing
  `hooks.json` changes Codex's hash, so the next `setup` reports `hook updated`.
- Probe hooks in a throwaway `CODEX_HOME`, never `~/.codex`. Codex runs them
  under `$SHELL -lc`, which can rebuild `PATH`: keep `bilbo` in
  `$HOME/.nix-profile/bin` of the throwaway `HOME`.

## Releases

Steps and the dist upgrade: `CONTRIBUTING.md#releases`. Two traps:
- Bump `version` in `Cargo.toml` and `plugins/bilbo/.codex-plugin/plugin.json`
  together, then run `nix develop -c cargo update --workspace`.
- Push only `v<version>` tags: the generated trigger also accepts `<version>`
  and `bilbo-v<version>`, and either would publish a second release.
