# Contributing to bilbo

bilbo is durable memory for coding agents: notes they write and recall, and a
library of sources they cite. To use it, start at [docs/](docs/README.md).
This page is for changing it; agents read [AGENTS.md](AGENTS.md) for the traps.
The GitHub wiki is a generated copy of `docs/` on main: edit the docs, never
the wiki.

## Set up

bilbo is a Rust CLI with a Nix flake. Run every command in the dev shell
(`nix develop`), which pins the toolchain (Rust 1.95.0) and the tools the
tests call. The vocabulary and the stack live in `openspec/config.yaml`.

## Verify a change

CI runs the same checks. The exact commands, including the Nix sandbox build
and the plugin validators, are in [AGENTS.md](AGENTS.md#commands). Format,
clippy with warnings denied and the tests must pass, and `nix flake check -L`
must pass when you touched `flake.nix` or a file the build reads (the flake
builds from a `lib.fileset`). A global `CARGO_TARGET_DIR` sends
`./target/debug/bilbo` elsewhere: point it at your checkout.

## Change behavior with OpenSpec

Behavior changes start as an OpenSpec change, not as code. `openspec/specs/`
is the current contract and changes only through `/opsx:archive` or
`/opsx:sync`, so every spec edit traces back to a reviewed change.

1. `/opsx:explore` to think the change through.
2. `/opsx:propose <name>` to draft the proposal, design, specs and tasks.
3. `/opsx:apply` once the proposal is approved.
4. `/opsx:archive` when it ships, or `/opsx:sync` to sync the specs sooner.

Codex runs them as `$openspec-explore`, `$openspec-propose`,
`$openspec-apply-change` and `$openspec-archive-change`. Git ignores the
archive folder, so the synced specs and the pull request are what the repo
keeps. A fix that leaves behavior unchanged (a typo, a refactor, a test-only
edit) needs no change.

## Architecture

`tests/layout.rs` checks the shape of `src/` and its failures say what to
change; the rules it cannot check are in
[AGENTS.md](AGENTS.md#architecture-rules).

## Write tests

- Unit tests live in the module they test and read fixtures through
  `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/...")`.
- CLI behavior is tested through the built binary with a clean environment,
  using the fakes in `tests/common/`: a fake embedder, and fake `claude`,
  `codex`, `launchctl`, `systemctl` and `llama-server` scripts that use only
  shell builtins.
- The test traps agents hit are in [AGENTS.md](AGENTS.md#gotchas).
  `tests/relay.rs` durability tests are the one place where the kill is the test.
- The full-disk test in `tests/relay.rs` runs only where `BILBO_TEST_TMPFS`
  names a size-limited tmpfs. Run it in a Linux container with `--tmpfs`.

CI runs on Linux. After touching `src/host/timer.rs`, `src/setup/`,
`tests/setup.rs` or the macOS branch of `src/host/swap.rs`, run their tests on
macOS. `tests/setup.rs` is several times slower there, because macOS scans
each freshly written script.

## Manual tests

After changing `Terminal` in `src/host/prompt.rs`, the one piece the unit
tests cannot reach, follow [docs/manual-tests.md](docs/manual-tests.md) under
`env -u AI_AGENT -u CLAUDE_CODE_CHILD_SESSION -u CODEX_THREAD_ID -u CODEX_CI`.

To try an edit under `plugins/` in Claude Code or Codex, install the plugin
from your checkout in a throwaway world from that page, with `claude` and
`codex` on its `PATH`: `$B setup --yes --no-timer --no-watch --plugin-source
<checkout>`. Both tools run a cached copy of the plugin, and `setup` keeps it
while the folder and the version are unchanged, so after each edit run
`$B setup --remove --yes`, then the same `setup` again. Never run `--remove`
outside a world: it also unloads your own timer, watcher and embedder.

## Fixtures

Never edit a fixture by hand.

- `tests/fixtures/device/` holds golden key and manifest files for fixed test
  seeds. Regenerate them with the ignored
  `identity::device::tests::write_fixtures`. Git keeps only the execute bit,
  so `tests/device.rs` sets the 0700 and 0600 modes itself.
- `tests/fixtures/agents/` holds recorded `claude` and `codex` output, the
  first line being the command. When a tool's JSON moves, re-record it against
  a throwaway `CLAUDE_CONFIG_DIR` and `CODEX_HOME`, and re-record
  `codex-app-server-*` when the app-server protocol moves. Trim
  `codex-app-server-config-read.txt` to `marketplaces`, `plugins`,
  `hooks.state` and their `origins`: Codex also returns every key and layer of
  the recording machine's managed config.
- `tests/fixtures/pages/` holds page fixtures. When an `htmd` bump changes the
  expected Markdown, review the diff and re-record the fixture.

To probe a Codex hook, use a throwaway `CODEX_HOME`, never `~/.codex`, and put
`bilbo` in `$HOME/.nix-profile/bin` of the throwaway `HOME`: Codex runs hooks
under `$SHELL -lc`, and a login profile can rebuild `PATH`.

## Releases

1. Bump `version` in `Cargo.toml` and `plugins/bilbo/.codex-plugin/plugin.json`
   together (`tests/plugin.rs` fails when they differ), then run
   `nix develop -c cargo update --workspace`.
   `plugins/bilbo/.claude-plugin/plugin.json` sets no version, so Claude Code
   follows commits.
2. Record the L1 baseline: build the release binary (`cargo build --release --locked`), run the test split once
   with `llama-server` and the pinned GGUF, commit `evals/l1-retrieval/baseline/<version>.json` and `.md` in the bump
   pull request, and `diff` it against the previous baseline (commands in `evals/l1-retrieval/README.md`).
3. Merge through a pull request.
4. Tag the commit on `main` that carries the bump `v<version>` and push the
   tag; the release workflow publishes the GitHub Release. Push only
   `v<version>` tags: the generated trigger also accepts `<version>` and
   `bilbo-v<version>`, and either would publish a second release.

To upgrade dist: change `cargo-dist-version` in `dist-workspace.toml` and run
`nix flake update nixpkgs-unstable` until `nix develop -c dist --version`
matches. Run `nix develop -c dist init --yes`, pin every action the new
`release.yml` names in `[dist.github-action-commits]` (commit from
`gh api repos/<owner>/<repo>/git/ref/tags/<tag>`; dereference an annotated
tag with `gh api repos/<owner>/<repo>/git/tags/<sha>`), run
`nix develop -c dist generate` again, then
`nix develop -c cargo test --locked --test workflows`.
