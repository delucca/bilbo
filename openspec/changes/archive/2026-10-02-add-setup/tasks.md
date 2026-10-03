# Tasks

Run every command inside `nix develop -c <cmd>` from the repo root, with `CARGO_TARGET_DIR` set to the checkout's `target`. Start only after add-distribution is archived: `test -f openspec/specs/distribution/spec.md`.

## 1. Foundations

- [x] 1.1 Add `xdg_state_home` to `store::Env`, read once in `main`, and add a `state_dir` resolver: absolute `XDG_STATE_HOME`, otherwise `~/.local/state`. Skip that part if add-note-digest already added it. Add a `config_home` resolver too: absolute `XDG_CONFIG_HOME`, otherwise `~/.config`, for the systemd unit folder. Verify with `cargo test --locked store::`
- [x] 1.2 Move `serde_json` from dev-dependencies to dependencies at the version `Cargo.lock` already holds. Update the `Stack` line in `openspec/config.yaml` and the dependency list in `AGENTS.md`. Verify with `cargo build --locked && git diff --stat Cargo.lock | (! grep -q .)`
- [x] 1.3 Add a config writer to `src/config.rs`: a header, then settings in key order, quoting and escaping by design.md's rule. Unit-test that a round trip through the existing parser holds for a plain URL, a value with trailing spaces, a value with a backslash, one starting with `"`, and the Qwen prefix with its newline. Also expose the config path resolution, the URL and variable-name checks, the local-URL test and the Qwen prefix default. Verify with `cargo test --locked config::`
- [x] 1.4 Add `src/command.rs`: the `Runner` trait, its `std::process::Command` implementation (stdin closed, environment inherited) and the PATH lookup of an executable file. Add `embed::Client::with_token` for a key already in memory and `embed::ollama_models` (`GET <url>/api/tags` with a 1 s limit). Unit-test the lookup and the Ollama parse. Verify with `cargo test --locked command:: embed::`

## 2. Plan and report (`setup`: Plan before writing, Step report, Reruns change nothing)

- [x] 2.1 Add `src/setup.rs` with the inputs, the `Plan` (steps × actions), `apply` and the report lines, plus the `setup` dispatch arm and USAGE line in `src/main.rs`. Parse every flag in `Setup flags`, with usage errors that never echo an unknown flag's value past its name, and make the answer flags imply non-interactive mode. Cover the `Modes` and `Setup flags` scenarios that need no terminal in `tests/setup.rs`, and the `cli` delta's verb-list scenario in `tests/cli.rs`. Verify with `cargo test --locked --test setup --test cli`
- [x] 2.2 Store and config steps: create the store, write a new config atomically, keep an existing one, keep it when the embedder flags equal its settings, refuse other embedder flags against an existing or managed config, and detect a managed config (a link, or a folder that is not writable). Cover the `Store step`, `New config file`, `Existing config file` (non-interactive) and `Managed config` scenarios, plus the `config` delta's `BILBO_CONFIG` scenarios. Verify with `cargo test --locked --test setup && cargo test --locked --test index --test recall`
- [x] 2.3 Embedder check in the plan stage, through `embed.rs` with a 15 s limit, run against the fake embedder from `tests/common/mod.rs`. Cover the non-interactive `Embedder check` scenarios (ok, 404, existing config not checked) and the `Secrets stay out of setup output` rule with a known token string. Verify with `cargo test --locked --test setup embedder`

## 3. Agent plugins (`setup`: Plugin source, Agent plugin step)

- [x] 3.1 Record the real JSON outputs into `tests/fixtures/agents/`. Run each command from design.md's table against throwaway `CLAUDE_CONFIG_DIR` and `CODEX_HOME` folders, including a failed add, and write the fake `claude` and `codex` scripts that print the same shapes and log their arguments. The tests write the fakes from `tests/common/fakes.rs` into a temporary folder that is the whole PATH, so the fakes keep their state in files and use only shell builtins. Verify with `ls tests/fixtures/agents | wc -l` showing at least one file per command in the table
- [x] 3.2 Add `src/agents.rs`: the runner, reading the marketplace and plugin lists, the "same source" rules, and the per-state command sequences from design.md. Codex refuses a different source too, so both tools remove the marketplace first. Add the plugin source rule (`share/bilbo` next to the resolved executable, else `delucca/bilbo#v<version>`, else `--plugin-source`). Unit-test the source rule with a temporary `<prefix>/bin` and `<prefix>/share/bilbo`. Cover every `Agent plugin step` and `Plugin source` scenario in `tests/setup.rs` with the fakes on PATH. Verify with `cargo test --locked agents:: && cargo test --locked --test setup plugin`

## 4. Index timer (`setup`: Index timer)

- [x] 4.1 Add `src/timer.rs`: the launchd plist and systemd unit texts as pure functions of the exe path, the interval, the log path and the carried locations, and the load, reload and unload command sequences. Unit-test the generated text for 15 and 30 minutes, with and without locations, and both platforms' command sequences with a scripted `Runner`. Verify with `cargo test --locked timer::`
- [x] 4.2 Wire the timer step into `apply`, with fake `launchctl` and `systemctl` on PATH and `HOME` in a temp folder. Cover the `Index timer` scenarios, including no embedder, no user manager, a moved binary, the carried locations, a key in a variable and turning the timer off. Verify with `cargo test --locked --test setup timer`
- [x] 4.3 Check the timer for real on macOS, outside the test suite: run `bilbo setup --yes` with a temporary `BILBO_HOME`, `BILBO_CONFIG` and `XDG_STATE_HOME` and a config pointing at bagend. Confirm `launchctl print gui/$(id -u)/io.github.delucca.bilbo.index` shows the agent, then run `bilbo setup --remove --yes`. Record it in `smoke.md`. Verify with `rg -q 'launchctl print' openspec/changes/add-setup/smoke.md`

## 5. Remove (`setup`: Remove)

- [x] 5.1 Add `--remove`: unload and delete the timer, uninstall and remove the plugin in each tool found, keep and name the store, config and key file, and refuse setup flags. Cover the `Remove` scenarios in `tests/setup.rs`. Verify with `cargo test --locked --test setup remove`

## 6. Wizard (`setup`: Modes, Embedder choices, Query prefix default, Embedder key, Embedder check, First index; `cli` Output streams)

- [x] 6.1 Add `cliclack = "0.5.6"` and `zeroize = "1.9.0"`. The planning round checked cliclack against Context7 and its source (design.md records it: stderr, `Interrupted`, no `ctrlc` handler); record any deviation found while building the adapter there. Verify with `cargo build --locked && rg -q '^name = "cliclack"' Cargo.lock`
- [x] 6.2 Add `src/wizard.rs` with the `Prompter` trait, its cliclack adapter, and the question flow:
  - the embedder choices, with Ollama detection (a 1 s `GET /api/tags`) and the model list;
  - the Qwen prefix default, behind an advanced-settings choice;
  - the key source, with masked paste into `Zeroizing`;
  - the agent multiselect and the interval;
  - the summary and confirmation, with the failure menu of the embedder check (retry, change, keyword only);
  - the first-index offer with its spinner.

  Unit-test every wizard scenario with a scripted `Prompter`, including Ctrl-C at each prompt writing nothing and the existing-key confirmation. Verify with `cargo test --locked wizard::`
- [x] 6.3 The key file: `create_new` with mode 0600 under a temporary name, then rename, with the config set to `embedder.token_file`. Only the wizard pastes a key, and the integration tests have no terminal, so unit-test it in `setup.rs`: the mode is 0600, a second write replaces the key, and a known key appears in no report or summary line. Keep the integration tests for the `--embedder-token-file` and `--embedder-token-env` key lines. Verify with `cargo test --locked --bin bilbo setup::tests::key && cargo test --locked --test setup key`, each showing at least one test passed

## 7. Home-manager module and docs

- [x] 7.1 Add `homeManagerModules.default` to `flake.nix` as design.md lays it out:
  - options, with `settings` as one option per known key, and the `token_env` assertion;
  - the config rendered with the Rust writer's quoting;
  - the activation entry setting the locations, `env -u BILBO_HOME -u BILBO_CONFIG`, the PATH for `launchctl` or `systemctl`, and passing `--claude`, `--codex` and the index flags;
  - `storeRoot`, exported as `BILBO_HOME` in the session.

  Add the `home-manager` input (release-26.05, following `nixpkgs`) and a flake check that evaluates a home-manager configuration with sample settings and asserts the rendered config text and the activation command, plus a misspelled key and a `token_env` key that must fail, and a disabled module that adds nothing. Verify with `nix flake check -L`
- [x] 7.2 Smoke-test the wizard in a real terminal on rivendell, driven through `/usr/bin/expect` with bagend's embedder reached through `ssh -L`, with a temporary `BILBO_HOME`, `BILBO_CONFIG`, `XDG_STATE_HOME`, `CLAUDE_CONFIG_DIR` and `CODEX_HOME`. Go through each path:
  - keyword only;
  - OpenAI with a pasted dummy key that is rejected with 401;
  - bagend over its tunnel, measuring the embedder check against the 15 s limit;
  - a rerun showing `kept` everywhere;
  - `--remove`.

  Record the transcript in `smoke.md`, with the key redacted. Verify with `test -s openspec/changes/add-setup/smoke.md`
- [x] 7.3 Update `AGENTS.md` and `README.md`:
  - `AGENTS.md`: the new modules (`setup`, `wizard`, `agents`, `timer`, `command`; only `setup` returns `Failure`), cliclack being confined to `wizard.rs`, `zeroize`, the `home-manager` input and the module, the fakes and fixtures, and `tests/setup.rs`;
  - `README.md`: `curl … | sh && bilbo setup`, the flags, `--remove`, the timer's key-file requirement, and the home-manager snippet with `storeRoot`.

  Verify with `for f in src/*.rs tests/*.rs; do rg -qF "$f" AGENTS.md || echo "missing $f"; done | (! grep .) && rg -q 'bilbo setup' README.md`

## 8. Integration

- [x] 8.1 Run the full suite and the flake checks. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
