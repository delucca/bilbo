# Tasks

## 1. The gate (batch A)

- [ ] 1.1 Add `console = "0.16.6"` to `Cargo.toml`, `("console", &["host/terminal.rs"])` to `PLACEMENT` in `tests/layout.rs`, and `console` to the Stack line of `openspec/config.yaml`: `nix develop -c cargo build --locked && nix develop -c cargo test --locked --test layout`
- [ ] 1.2 Give `shared::store::Env` the fields `ai_agent`, `claude_code_child_session`, `codex_thread_id`, `codex_ci`, `no_color`, `clicolor`, `term`, `columns` and `lang`, drop `claudecode`, add `Env::agent()`, and move every hand-built `Env` in the unit tests to `..Env::from_vars(|_| None)`: `nix develop -c cargo test --locked --bin bilbo shared::store`
- [ ] 1.3 Switch `src/identity/device.rs`, `src/identity/pair/mod.rs` and `src/setup/facts.rs` to `env.agent()`, delete their `marked`, and move `tests/device.rs`, `tests/pair.rs` and `src/setup/driven.rs` to `CLAUDE_CODE_CHILD_SESSION`, `AI_AGENT` and `CODEX_CI`, with a `CLAUDECODE`-only case that is a person: `nix develop -c cargo test --locked --test device --test pair && nix develop -c cargo test --locked --bin bilbo identity setup`
- [ ] 1.4 Write `src/host/terminal.rs` (`Term`, `Stream`, `open`, `decide`, `init`, `Tone`, `paint`, `Mark`, `glyph`, `mark`, `marked`, `width_of`, `cut`, `wrap`, `fold`, `pad`, `table`, `tilde`, `ago`, `group`, `size`, `count`, and the test-only `fixed` and `styled`) with its unit tests, declare it in `src/host/mod.rs`, and add `frontmatter::created_time` for `ago`: `nix develop -c cargo test --locked --bin bilbo host::terminal shared::frontmatter`
- [ ] 1.5 In `src/main.rs`, decide a `Term` per stream before dispatch, call `terminal::init`, add `Level` and the leveled `print_stderr`, print usage errors and refusals by level, add `Failure::Unmatched`, print warnings after a human stdout, route help through the gate and delete `styled`, and update the overview's `Diagnostics` sentence: `nix develop -c cargo test --locked --bin bilbo tests::`
- [ ] 1.6 Add the pseudo-terminal helper `bilbo_tty` to `tests/common/mod.rs` and the gate tests to `tests/cli.rs`: `nix develop -c cargo test --locked --test cli`
- [ ] 1.7 Make the plain-byte plurals: `1 passage not indexed`, `1 note` in `scope`, `sync` and setup's sync line, `1 heading` in the facts line, with their tests, the recall skill's line and `docs/troubleshooting.md`: `nix develop -c cargo test --locked --test recall --test scope --test sync --test setup --test library`
- [ ] 1.8 Keep the source's own words in `cite`'s `nearest passage:` hint, with unit tests: `nix develop -c cargo test --locked --bin bilbo citation && nix develop -c cargo test --locked --test cite`
- [ ] 1.9 Update `AGENTS.md`'s rule on the `bilbo: ` prefix: `nix develop -c cargo fmt --check && nix develop -c cargo clippy --locked --all-targets -- -D warnings && nix develop -c cargo test --locked`

## 2. recall (batch B)

- [ ] 2.1 Add `markdown::plain` to `src/shared/markdown.rs`, the display flattening of a passage, with unit tests: `nix develop -c cargo test --locked --bin bilbo shared::markdown`
- [ ] 2.2 Give `search::recall::Output` its hits and total, return `Failure::Unmatched` when nothing matches, and write `view` for notes and library hits with snippet windowing, match bolding, cutting and wrapping at widths 100 and 50, the count line and the hints, with unit tests: `nix develop -c cargo test --locked --bin bilbo search::recall && nix develop -c cargo test --locked --test recall`
- [ ] 2.3 Wire recall's view in `src/main.rs` and add its terminal tests to `tests/recall.rs`: `nix develop -c cargo test --locked --test recall --test cli`

## 3. check, setup and the tables (batch C)

- [ ] 3.1 Add `terminal::Step`, `steps` and `tilde_text`; give `check::Output` its problems by file and its counts, and write its `view`, with unit tests: `nix develop -c cargo test --locked --bin bilbo check && nix develop -c cargo test --locked --test check`
- [ ] 3.2 Give `setup::Outcome` its steps and write the report's `view`, with unit tests, and wire it after the wizard too: `nix develop -c cargo test --locked --bin bilbo setup && nix develop -c cargo test --locked --test setup`
- [ ] 3.3 Write the `view` of `scope`'s listing: `nix develop -c cargo test --locked --bin bilbo note::scope && nix develop -c cargo test --locked --test scope`
- [ ] 3.4 Write the `view` of `library`'s corpus list: `nix develop -c cargo test --locked --bin bilbo library::cli && nix develop -c cargo test --locked --test library`
- [ ] 3.5 Write the `view` of `device`, `device list` and the init and recover step report: `nix develop -c cargo test --locked --bin bilbo identity::device && nix develop -c cargo test --locked --test device`

## 4. The other verbs (batch D)

- [ ] 4.1 Write the `view` of `sync`'s status report: `nix develop -c cargo test --locked --bin bilbo sync::cli && nix develop -c cargo test --locked --test sync`
- [ ] 4.2 Write the `view` of `history`'s list and diff and of `restore`: `nix develop -c cargo test --locked --bin bilbo note::history note::restore && nix develop -c cargo test --locked --test history --test restore`
- [ ] 4.3 Write the `view` of a guide and of `library show`: `nix develop -c cargo test --locked --bin bilbo library::cli && nix develop -c cargo test --locked --test library`
- [ ] 4.4 Write the `view` of `new` and `index`: `nix develop -c cargo test --locked --bin bilbo note::new search::index && nix develop -c cargo test --locked --test new --test index`

## 5. Docs and the end-to-end pass (batch E)

- [ ] 5.1 Update `docs/reference/commands.md`, `docs/reference/configuration.md`, `docs/guides/{devices,scopes,sync,library}.md` and `docs/troubleshooting.md`: `python3 .github/wiki.py docs "$(mktemp -d)" https://example.com/repo && nix develop -c cargo test --locked --test cli`
- [ ] 5.2 Add the terminal pass to `docs/manual-tests.md` and run it, with and without `NO_COLOR`, `AI_AGENT=x`, `TERM=dumb` and `COLUMNS=60`, and compare the piped bytes of every verb with the previous release and check the terminal runs: `sh <work>/e2e/compare.sh && sh <work>/e2e/tty.sh`
- [ ] 5.3 Validate the plugin and the change: `claude plugin validate . && claude plugin validate plugins/bilbo && openspec validate style-output --strict`
- [ ] 5.4 Run the full verification: `nix develop -c cargo fmt --check && nix develop -c cargo clippy --locked --all-targets -- -D warnings && nix develop -c cargo test --locked && nix flake check -L`
