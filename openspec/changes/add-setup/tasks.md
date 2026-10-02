# Tasks

Run every command inside `nix develop -c <cmd>` from the repo root, with `CARGO_TARGET_DIR` set to the checkout's `target`. Start only after add-distribution is archived: `test -f openspec/specs/distribution/spec.md`.

## 1. Foundations

- [ ] 1.1 Add `xdg_state_home` to `store::Env`, read once in `main`, and add a `state_dir` resolver: absolute `XDG_STATE_HOME`, otherwise `~/.local/state`. Skip this if add-note-digest already added it. Verify with `cargo test --locked store::`
- [ ] 1.2 Move `serde_json` from dev-dependencies to dependencies at the version `Cargo.lock` already holds. Update the `Stack` line in `openspec/config.yaml` and the dependency list in `AGENTS.md`. Verify with `cargo build --locked && git diff --stat Cargo.lock | (! grep -q .)`
- [ ] 1.3 Add a config writer to `src/config.rs`: a header, then settings in key order, quoting and escaping only when needed. Unit-test that a round trip through the existing parser holds for a plain URL, a value with trailing spaces and the Qwen prefix with its newline. Verify with `cargo test --locked config::`

## 2. Plan and report (`setup`: Plan before writing, Step report, Reruns change nothing)

- [ ] 2.1 Add `src/setup.rs` with the inputs, the `Plan` (steps × actions), `apply` and the report lines, plus the `setup` dispatch arm and USAGE line in `src/main.rs`. Parse every flag in `Setup flags`, with usage errors that never echo an unknown flag's value past its name. Cover the `Modes` and `Setup flags` scenarios that need no terminal in `tests/setup.rs`, and the `cli` delta's verb-list scenario in `tests/cli.rs`. Verify with `cargo test --locked --test setup --test cli`
- [ ] 2.2 Store and config steps: create the store, write a new config atomically, keep an existing one, refuse embedder flags against an existing or managed config, and detect a managed config (a link, or a folder that is not writable). Cover the `Store step`, `New config file`, `Existing config file` (non-interactive) and `Managed config` scenarios, plus the `config` delta's `BILBO_CONFIG` scenarios. Verify with `cargo test --locked --test setup && cargo test --locked --test index --test recall`
- [ ] 2.3 Embedder check in the plan stage, through `embed.rs` with a 15 s limit, run against the fake embedder from `tests/common/mod.rs`. Cover the non-interactive `Embedder check` scenarios (ok, 404, existing config not checked) and the `Secrets stay out of setup output` rule with a known token string. Verify with `cargo test --locked --test setup embedder`

## 3. Agent plugins (`setup`: Plugin source, Agent plugin step)

- [ ] 3.1 Record the real JSON outputs into `tests/fixtures/agents/`. Run each command from design.md's table against throwaway `CLAUDE_CONFIG_DIR` and `CODEX_HOME` folders, including a failed add, and write the fake `claude` and `codex` scripts that replay them and log their arguments. Verify with `ls tests/fixtures/agents | wc -l` showing at least one file per command in the table
- [ ] 3.2 Add `src/agents.rs`: the runner, reading the marketplace and plugin lists, the "same source" rules, and the per-state command sequences from design.md. Add the plugin source rule (`share/bilbo` next to the resolved executable, else `delucca/bilbo#v<version>`, else `--plugin-source`). Unit-test the source rule with a temporary `<prefix>/bin` and `<prefix>/share/bilbo`. Cover every `Agent plugin step` and `Plugin source` scenario in `tests/setup.rs` with the fakes on PATH. Verify with `cargo test --locked agents:: && cargo test --locked --test setup plugin`

## 4. Index timer (`setup`: Index timer)

- [ ] 4.1 Add `src/timer.rs`: the launchd plist and systemd unit texts as pure functions of the exe path, the interval and the log path, and the load, reload and unload command sequences. Unit-test the generated text for 15 and 30 minutes. Verify with `cargo test --locked timer::`
- [ ] 4.2 Wire the timer step into `apply`, with fake `launchctl` and `systemctl` on PATH and `HOME` in a temp folder. Cover the `Index timer` scenarios, including no embedder, no user manager and a moved binary. Verify with `cargo test --locked --test setup timer`
- [ ] 4.3 Check the timer for real on macOS, outside the test suite: run `bilbo setup --yes` with a temporary `BILBO_HOME`, `BILBO_CONFIG` and `XDG_STATE_HOME` and a config pointing at bagend. Confirm `launchctl print gui/$(id -u)/io.github.delucca.bilbo.index` shows the agent, then run `bilbo setup --remove --yes`. Record it in `smoke.md`. Verify with `rg -q 'launchctl print' openspec/changes/add-setup/smoke.md`

## 5. Remove (`setup`: Remove)

- [ ] 5.1 Add `--remove`: unload and delete the timer, uninstall and remove the plugin in each tool found, keep and name the store, config and key file, and refuse setup flags. Cover the `Remove` scenarios in `tests/setup.rs`. Verify with `cargo test --locked --test setup remove`

## 6. Wizard (`setup`: Modes, Embedder choices, Query prefix default, Embedder key, Embedder check, First index; `cli` Output streams)

- [ ] 6.1 Pull the current cliclack 0.5.6 docs with `use-context7`. Confirm three things: it draws on stderr, Ctrl-C and Esc return `Interrupted`, and whether a `ctrlc` handler is needed to restore the cursor. Add `cliclack = "0.5.6"` and record any deviation from design.md there. Verify with `cargo build --locked && rg -q '^name = "cliclack"' Cargo.lock`
- [ ] 6.2 Add `src/wizard.rs` with the `Prompter` trait, its cliclack adapter, and the question flow:
  - the embedder choices, with Ollama detection (a 1 s `GET /api/tags`) and the model list;
  - the Qwen prefix default, behind an advanced-settings choice;
  - the key source, with masked paste into `Zeroizing`;
  - the agent multiselect and the interval;
  - the summary and confirmation, with the failure menu of the embedder check (retry, change, keyword only);
  - the first-index offer with its spinner.

  Unit-test every wizard scenario with a scripted `Prompter`, including Ctrl-C at each prompt writing nothing and the existing-key confirmation. Verify with `cargo test --locked wizard::`
- [ ] 6.3 The key file: `create_new` with mode 0600 under a temporary name, then rename, with the config set to `embedder.token_file`. Test that the mode is 0600 and that the key appears in no output. Verify with `cargo test --locked --test setup key`

## 7. Home-manager module and docs

- [ ] 7.1 Add `homeManagerModules.default` to `flake.nix` as design.md lays it out:
  - options, with an assertion on unknown `settings` keys;
  - the config rendered with the Rust writer's quoting;
  - the activation entry passing `--claude` and `--codex` and the index flags.

  Add a flake check that evaluates a home-manager configuration with sample settings and asserts the rendered config text and the activation command, plus one with a misspelled key that must fail. Verify with `nix flake check -L`
- [ ] 7.2 Smoke-test the wizard in a real terminal on rivendell, with a temporary `BILBO_HOME`, `BILBO_CONFIG`, `XDG_STATE_HOME`, `CLAUDE_CONFIG_DIR` and `CODEX_HOME`. Go through each path:
  - keyword only;
  - OpenAI with a pasted dummy key that is rejected with 401;
  - bagend over its tunnel, measuring the embedder check against the 15 s limit;
  - a rerun showing `kept` everywhere;
  - `--remove`.

  Record the transcript in `smoke.md`, with the key redacted. Verify with `test -s openspec/changes/add-setup/smoke.md`
- [ ] 7.3 Update `AGENTS.md` and `README.md`:
  - `AGENTS.md`: the new modules (`setup`, `wizard`, `agents`, `timer`; only `setup` returns `Failure`), cliclack being confined to `wizard.rs`, the fakes and fixtures, and `tests/setup.rs`;
  - `README.md`: `curl … | sh && bilbo setup`, the flags, `--remove`, and the home-manager snippet.

  Verify with `for f in src/*.rs tests/*.rs; do rg -qF "$f" AGENTS.md || echo "missing $f"; done | (! grep .) && rg -q 'bilbo setup' README.md`

## 8. Integration

- [ ] 8.1 Run the full suite and the flake checks. Verify with `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && nix flake check -L`
