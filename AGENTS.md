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
  stderr line with `bilbo: `. The one exception is `Terminal` in
  `src/host/prompt.rs`, which cliclack draws on stderr without the prefix.
- `src/` is one folder per domain, each verb inside the domain it serves:
  `note/`, `search/`, `library/`, `citation/`, `identity/`, `sync/`, `relay/`,
  `setup/` and `host/`. A domain's `mod.rs` holds its `//!` summary and its
  `mod` lines; `note/` and `citation/` also keep their model there, and
  `setup/` and `relay/`, which are themselves verbs, keep the verb's root. `src/main.rs` parses arguments, owns
  `Failure`, dispatches to `<domain>::<verb>::run` (except `setup::run`,
  `relay::run` and `check::run`) and prints.
- `src/shared/` is the Shared Kernel: `store`, `markdown`, `frontmatter`,
  `text`, `config` and `hash`. A module joins it only when two domains use
  it, and `shared/` never imports a domain. Callers keep module-qualified
  calls: `use crate::shared::store;`, then `store::root(..)`.
- The verbs are `note/new.rs`, `note/watch.rs`, `note/history.rs`,
  `note/restore.rs`, `note/scope.rs`, `search/recall.rs`, `search/index.rs`,
  `search/digest.rs`, `library/cli/`, `citation/cite.rs`,
  `identity/device.rs`, `identity/pair/`, `sync/cli.rs`, `relay/`, `setup/` and `src/check.rs`, which spans note and
  library and so stays at the root. A verb parses its own arguments, returns `crate::Failure` and never prints,
  and only `main` uses a verb's module. `watch`, `pair` and `relay`, like `setup`, take
  callbacks for their lines and `main` prints each one. `digest` is the
  exception: it returns a `digest::Outcome` (lines and one diagnostic) and
  `main` always exits 0 for it, because a prompt hook that exits 2 blocks the
  prompt. Code outside the verbs returns plain values and `String` messages
  and never names `Failure`; it may use other domains without forming a
  cycle (today `search` uses `note` and `library`, `identity` uses `host`, and
  `sync` uses `note`, `identity` and `host`, and `relay` uses `sync` and
  `identity`). The library modules of `identity`
  (`keys`, `phrase`, `manifest`, `ceremony`, `pake`) never use a domain that uses
  `identity`, so later domains can build on them. Only verbs call into
  `sync` from `note` and `identity` (`note/restore.rs`, `note/scope.rs`,
  `identity/device.rs`, `identity/pair/`); `tests/layout.rs` skips verb files in its cycle
  check, so a non-verb file of `note` or `identity` that uses `sync` closes a
  cycle and fails it.
  `identity/script.rs`, the scripted `Prompter`, is test-only. Nothing outside
  `main` uses `relay/` except test code: the unit tests of `sync/remote/` start
  a relay with the test-only `relay::start`, and `tests/layout.rs` reads only
  the code before `mod tests`. `httparse` is used in `relay/http.rs` alone.
- A module with children is `foo/mod.rs`, never `foo.rs` beside `foo/`
  (clippy's `self_named_module_files`, enabled in `src/main.rs`). Items are
  `pub` or private, never `pub(crate)`. Name modules by absolute `crate::`
  paths; only the files inside a verb's folder reach its `mod.rs` and their
  siblings through `super::`. No re-exports, and no glob imports outside test code.
- `tests/layout.rs` checks these rules, and its `PLACEMENT` table keeps each
  listed crate in its listed files (`tests/common` also uses `sha2`, to write
  library files with a correct digest). A change that adds a crate, a
  domain, a verb or a shared module edits that file, and justifies a new
  crate in its `design.md`.
- A new verb is a file in its domain (`src/<domain>/<verb>.rs`, or a folder
  with a `mod.rs` once it outgrows one file; never a module named like its
  domain, which clippy's `module_inception` rejects), its `pub mod` line in
  the domain's `mod.rs`, its path in `VERBS` in `tests/layout.rs`, its
  dispatch arm and USAGE line in `src/main.rs`, `tests/<verb>.rs`, its own
  capability spec, and a MODIFIED `cli` spec (its Verb dispatch requirement
  lists the verbs).
- A new `library` subcommand is `src/library/cli/<word>.rs`, its arm in
  `run` in `src/library/cli/mod.rs`, and its word in
  `library::corpus::RESERVED`, or a corpus of that name shadows it.
- Unit tests live in the module they test and move with it. A unit test
  reads a fixture under `tests/fixtures/` through
  `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/...")`, never a path
  relative to its source file. CLI behavior is tested through
  the built binary with a clean environment, using the fakes in
  `tests/common/` (a fake embedder, and fake `claude`, `codex`, `launchctl`,
  `systemctl` and `llama-server` scripts that use only shell builtins).
- Tests stay offline. A test that gets past planning with `--embedder-local`
  calls `place_model` first; only the `#[ignore]`
  `host::model::tests::pinned_url_honors_a_range` reaches Hugging Face.
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
- CI runs on Linux, so it skips the macOS-only tests in `src/host/timer.rs`,
  `src/setup/` and `tests/setup.rs`. After touching launchd code, run them on
  macOS. `tests/setup.rs` runs several times slower there, because macOS
  scans each freshly written script.
- Tests of `bilbo watch` run it as a child process and poll with a deadline.
  A check that nothing was recorded waits on a barrier, a later edit to another
  note that it polls for, never a fixed sleep.
- On macOS a child spawned while another thread is creating a pipe inherits
  that pipe (std has no `pipe2` there). A test that runs `bilbo` from a second
  thread drops its `Watcher` before joining that thread, or the join waits
  forever (`history_does_not_block_a_starting_watcher`).
- `tests/sync.rs` runs two `bilbo watch` children over one temporary folder
  and polls with a deadline, never a fixed sleep; a check that nothing
  arrived waits on a barrier, a later note that it polls for.
- `tests/relay.rs` runs the built `bilbo relay --listen 127.0.0.1:0` through
  `tests/common::Relay` and reads the port from its first stderr line. Its
  durability tests (a SIGKILL mid-body, a SIGKILL after the answer) also run
  on macOS, where `sync_all` is `F_FULLFSYNC`. The full-disk test runs only
  where `BILBO_TEST_TMPFS` names a size-limited tmpfs, and skips otherwise;
  run it in a Linux container with `--tmpfs`.
- `sync::integrate`'s crash points and races are tested through its `Step`
  hook, which stops an inbound write where a kill would, never by killing a
  child.
- Restore's crash points are tested through its step hook, which stops it where
  a kill would, and `scope set`'s races through its exchange hook. Do not test
  them by killing a child.
- CI runs only the Linux branch of `src/host/swap.rs`. After touching it, run
  its tests on macOS.
- `config::is_local` calls `0.0.0.0` remote, and a connect to it lands on
  `common::Fake`'s `127.0.0.1` listener, so a test that needs a remote embedder
  uses `http://0.0.0.0:<fake.port()>`. `zero_address` in `tests/index.rs` proves it.
- `Terminal` in `src/host/prompt.rs` is the only code the unit tests cannot
  reach. After changing it, repeat the expect runs recorded in the
  `smoke.md` of `openspec/changes/archive/2026-10-02-add-setup/`,
  `2026-10-03-add-local-embedder/` and `2026-10-05-add-device-keys/` (run
  the last under `env -u CLAUDECODE -u CODEX_THREAD_ID`).
- `tests/fixtures/device/` holds golden key and manifest files for fixed test
  seeds. Regenerate them with the ignored
  `identity::device::tests::write_fixtures`, never by hand. Git keeps only the
  execute bit, so `tests/device.rs` sets the 0700 and 0600 modes itself. Binary
  tests run without a terminal and never feed the phrase through a hook; the
  ceremony is tested in `identity/ceremony.rs` and `identity/device.rs` with the
  scripted prompter.
- The pairing exchange is tested in one process, through `pair::run`'s injected
  terminal flag, answer and limits, never through a hook. `tests/pair.rs` runs
  the binary without a terminal, so it reaches only the refusals before the
  mailbox.
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
