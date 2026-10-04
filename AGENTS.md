# bilbo

Durable memory for coding agents: notes they write and recall, and a library
of sources they cite. Product frame, vocabulary and stack live in
`openspec/config.yaml`; install and setup in `README.md`.

## Commands

Run everything in the dev shell, with the target folder in this checkout: a
global `CARGO_TARGET_DIR` moves `./target/debug/bilbo` elsewhere.

```sh
export CARGO_TARGET_DIR="$PWD/target"
# Verification (the `Verification` line of openspec/config.yaml; CI's verify job)
nix develop -c cargo fmt --check
nix develop -c cargo clippy --locked --all-targets -- -D warnings
nix develop -c cargo test --locked
# The package with its tests in the Nix sandbox, and the home-manager module check (CI's nix job)
nix flake check -L
# Recall and digest speed tests, ignored by default
nix develop -c cargo test --release --test recall -- --ignored
nix develop -c cargo test --release --test digest -- --ignored
# After touching plugins/ (one missing-version warning each is expected; never --strict)
claude plugin validate . && claude plugin validate plugins/bilbo
PYTHONDONTWRITEBYTECODE=1 python3 ~/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py plugins/bilbo
```

## Workflow

- Start every behavior change with an OpenSpec change: `/opsx:explore` to
  think it through, `/opsx:propose <name>` to draft it, `/opsx:apply` once
  the user approves it, `/opsx:archive` when it ships. Codex runs the same
  workflows as `$openspec-explore`, `$openspec-propose`,
  `$openspec-apply-change` and `$openspec-archive-change`.
- `openspec/specs/` is the current contract. Change it only through
  `/opsx:archive` or `/opsx:sync`, so every spec edit traces back to a
  reviewed change.
- Make fixes that leave behavior unchanged (typos, refactors, test-only
  edits) directly, without a change.
- Update this file when a change adds a command, a generated file or a rule
  that the code, the specs and the README cannot show. Describe modules in
  their specs and code, not here.

## Architecture rules

- `src/main.rs` is the only writer of stdout and stderr, and prefixes every
  stderr line with `bilbo: `. The one exception is the setup wizard, which
  cliclack draws on stderr without the prefix.
- Library modules (`store`, `note`, `rank`, `config`, `embed`, `model`,
  `vectors`, `command`, `agents`, `timer`, `source`, `corpus`, `hash`, `text`,
  `citation`, `plan`, `fetch`, `html`) return plain values and `String`
  messages: they never print and never return `Failure`. Verbs build on them,
  never on each other, return `crate::Failure` and never print. `digest` is
  the exception: it returns a `digest::Outcome` (lines and one diagnostic) and
  `main` always exits 0 for it, because a prompt hook that exits 2 blocks the
  prompt.
- A new verb is `src/<verb>.rs`, its `mod` line, dispatch arm and USAGE line
  in `src/main.rs`, `tests/<verb>.rs`, its own capability spec, and a
  MODIFIED `cli` spec (its Verb dispatch requirement lists the verbs).
- Keep each dependency in its one user: `cliclack` and `libc` in
  `src/wizard.rs`, `ring` in `src/model.rs`, `sha2` in `src/hash.rs`,
  `unicode-normalization` in `src/text.rs`, `htmd` and `markup5ever_rcdom` in
  `src/html.rs`. Justify a new one in the change's `design.md`.
- Unit tests live in the module they test. CLI behavior is tested through
  the built binary with a clean environment, using the fakes in
  `tests/common/` (a fake embedder, and fake `claude`, `codex`, `launchctl`,
  `systemctl` and `llama-server` scripts that use only shell builtins).
- Tests stay offline. A test that gets past planning with `--embedder-local`
  calls `place_model` first; only the `#[ignore]`
  `model::tests::pinned_url_honors_a_range` reaches Hugging Face.
- Plugin skill frontmatter uses only `name`, `description`, `license` and
  `allowed-tools`, the keys both Claude Code and Codex accept.
- Pin every dependency exactly: Cargo through the committed `Cargo.lock`,
  other ecosystems in the manifest; actions in `.github/workflows/` by
  40-hex commit SHA (`tests/workflows.rs` enforces it).

## Generated files

- `.claude/commands/opsx/`, `.claude/skills/openspec-*` and
  `.agents/skills/openspec-*` come from `openspec update`, using OpenSpec
  1.14.0 (their `generatedBy`; older releases lack subcommands the
  workflows call). Regenerate them after a version bump instead of editing
  them. Keep the two skill folders as separate copies: each tool gets its
  own wording, and a symlink makes every `openspec update` rewrite them.
- `.agents/plugins/marketplace.json` is written by hand.
- `.github/workflows/release.yml` comes from `dist generate`, from
  `dist-workspace.toml`. Regenerate it instead of editing it: the release
  `plan` job runs `dist plan` on every PR and fails when the file is stale.

## Gotchas

- `land` and `check` treat any hidden entry in `<root>/library/` as absent, and
  the `land` lock, `<root>/library/.lock`, is one: never list or remove it as a
  stray file.
- `flake.nix` builds from a `lib.fileset`: a new file the build or the
  tests read must join it, or `nix flake check` fails while cargo passes.
- CI reruns the tests with the version bumped to 99.99.99. Read the version
  from `env!("CARGO_PKG_VERSION")` in tests, never a literal.
- CI and the dev shell run Rust 1.95.0, the `rust-version` floor. Bump the
  toolchain together with `rust-version`. A `flake.lock` bump can bring a
  newer clippy that flags lints CI does not.
- CI runs on Linux, so it skips the macOS-only tests in `src/timer.rs`,
  `src/setup.rs` and `tests/setup.rs`. After touching launchd code, run them
  on macOS. `tests/setup.rs` runs several times slower there, because macOS
  scans each freshly written script.
- The wizard's `Terminal` adapter is the only code the unit tests cannot
  reach. After changing it, repeat the expect runs recorded in the
  `smoke.md` of `openspec/changes/archive/2026-10-02-add-setup/` and
  `2026-10-03-add-local-embedder/`.
- `tests/fixtures/agents/` holds recorded `claude` and `codex` output, the
  first line being the command. When a tool's JSON moves, re-record them
  against throwaway `CLAUDE_CONFIG_DIR` and `CODEX_HOME`.
- A skill's heredoc body is data, not commands: `tests/plugin.rs` skips the
  lines from one ending in `<<'EOF'` to the line `EOF` when it checks each
  `bash` line against `allowed-tools`. Use that exact delimiter.
- Every command in `plugins/bilbo/hooks/hooks.json` must never pass on
  bilbo's exit code (`; exit 0`: an older `bilbo` exits 2 on the unknown verb)
  and must stay quiet without `bilbo` on `PATH`.
- On an auto-compaction, neither Claude Code nor Codex passes PreCompact or
  PostCompact output to the model. SessionStart with the matcher `compact` is
  the event that does, in both (`openspec/changes/archive/2026-10-03-add-note-skill/probe.md`,
  the path the archive gives it).
- Codex runs a plugin hook only once it is trusted, and `codex exec` skips an
  untrusted one in silence. `bilbo setup` trusts it through `codex app-server`
  (the fake `codex` imitates it), never by editing `config.toml`. Changing
  `hooks.json` changes Codex's hash, so the next `setup` reports `hook
  updated`. Probe hooks in a throwaway `CODEX_HOME`, never `~/.codex`, and
  re-record `tests/fixtures/agents/codex-app-server-*` when the protocol moves.
- Codex runs hooks under `$SHELL -lc`, and a login profile can rebuild `PATH`.
  A hook probe needs `bilbo` in a folder that profile keeps, such as
  `$HOME/.nix-profile/bin` of the throwaway `HOME`.
- When an `htmd` bump changes the expected Markdown of a page fixture under
  `tests/fixtures/pages/`, review the diff and re-record the fixture.

## Releases

1. Bump `version` in `Cargo.toml` and `plugins/bilbo/.codex-plugin/plugin.json`
   together (`tests/plugin.rs` fails when they differ), then run
   `nix develop -c cargo update --workspace`. `.claude-plugin/plugin.json`
   sets no version, so Claude Code follows commits.
2. Merge through a pull request.
3. Tag the commit on `main` that carries the bump `v<version>` and push the
   tag; the release workflow publishes the GitHub Release. Push only
   `v<version>` tags: the generated trigger also accepts `<version>` and
   `bilbo-v<version>`, and either would publish a second release.

To upgrade dist: change `cargo-dist-version` and run
`nix flake update nixpkgs-unstable` until `nix develop -c dist --version`
matches. Run `nix develop -c dist init --yes`, pin every action the new
`release.yml` names in `[dist.github-action-commits]` (commit from
`gh api repos/<owner>/<repo>/git/ref/tags/<tag>`; dereference an annotated
tag with `gh api repos/<owner>/<repo>/git/tags/<sha>`), run
`nix develop -c dist generate` again, then
`nix develop -c cargo test --locked --test workflows`.
