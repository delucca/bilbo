# Tasks

Run every command below inside `nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#clippy nixpkgs#rustfmt`, from the repo root (see design.md, Toolchain).

## 1. Scaffold

- [ ] 1.1 Create `Cargo.toml` (package and binary `bilbo`, `edition = "2024"`, `rust-version = "1.95"`, dependency `jiff = "0.2.37"`), a `src/main.rs` that exits 0, and a `.gitignore` holding `/target`. Verify with `cargo build && test -f Cargo.lock`
- [ ] 1.2 Fill in `Stack` (Rust, one crate, binary `bilbo`, `jiff` the only dependency) and `Verification` (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`) in `openspec/config.yaml`. Verify with `rg -n '^  (Stack|Verification):' openspec/config.yaml`

## 2. Command line (`cli` spec)

- [ ] 2.1 In `src/main.rs`: verb dispatch for `new` and `check`, usage on stderr with exit 2 for no verb, an unknown verb or an unknown option, `--help`/`-h` on stdout with exit 0, and the `bilbo: ` prefix on every diagnostic. Add a shared test helper in `tests/common/mod.rs` (a unique temp folder under `std::env::temp_dir()`, removed on drop, and a runner over `env!("CARGO_BIN_EXE_bilbo")` with a clean environment). Cover every `cli` scenario in `tests/cli.rs`. Verify with `cargo test --test cli`

## 3. Store and note format (`note-store` spec)

- [ ] 3.1 In `src/store.rs`: root resolution (`BILBO_HOME`, then an absolute `XDG_DATA_HOME`, then `$HOME/.local/share/bilbo`, and a relative `BILBO_HOME` as a usage error), and listing `notes/` with hidden entries skipped. Unit-test every store root scenario with the environment passed in, not read globally. Verify with `cargo test store::`
- [ ] 3.2 In `src/note.rs`: the nine kinds, parsing a filename into kind and topic, topic validation, ULID minting from `/dev/urandom` and canonical-form validation, and formatting and validating `created` with jiff `strftime`/`strptime` on `%Y-%m-%dT%H:%M%:z`. Unit-test the name, id and created scenarios, including `2026-02-30T10:00-03:00`, the seconds form and the `Z` form. Verify with `cargo test note::`
- [ ] 3.3 In `src/note.rs`: the strict frontmatter reader (the `---` delimiters, `id`/`created`/`sources` once each, unknown keys, the exact item form `  - "<type>: <value>"`, `sources: []` and an empty `sources:`) and title counting outside fenced blocks. Return every problem with its line, plus a renderer for new notes. Unit-test the frontmatter, sources and title scenarios, and that each rendered note reads back with no problems. Verify with `cargo test note::`

## 4. `bilbo new` (`note-create` spec)

- [ ] 4.1 In `src/new.rs`: argument checks (missing or unknown kind with the nine kinds listed, invalid topic, empty or multi-line `--title`) before any folder is created, the default title from the topic, the cross-kind topic check, creating the root and `notes/`, and printing the absolute path. Cover every create, content and argument scenario in `tests/new.rs`, including a `new` followed by `check` exiting 0. Verify with `cargo test --test new`
- [ ] 4.2 Add the atomic create: write `notes/.new-<id>.tmp`, `fsync`, `hard_link` to the target, remove the temp file, and treat `EEXIST` as a refusal (exit 1) that leaves the existing file's bytes unchanged. Add the two-process race test, the existing-file test and the unwritable-folder test to `tests/new.rs`. Verify with `cargo test --test new`

## 5. `bilbo check` (`store-check` spec)

- [ ] 5.1 In `src/check.rs`: a missing `notes/` gives `bilbo: no store at <root>` with exit 1; otherwise apply the name, subfolder, frontmatter, id, created, sources and title rules to every entry, collecting every problem. Print `<path relative to the root>: <message>` lines sorted by path and then message, with exit 1 when there is any problem and 0 otherwise. Cover the report, every-problem and missing-store scenarios in `tests/check.rs` with stores built in temp folders. Verify with `cargo test --test check`
- [ ] 5.2 Add cross-file detection: a shared id and a shared topic are reported on each file involved, naming the other file. Add the read-only test, which snapshots the bytes and modification times of every entry before and after a run on a store with problems. Verify with `cargo test --test check`

## 6. Architecture in `AGENTS.md`

- [ ] 6.1 Once the code is in, add an `## Architecture` section to `AGENTS.md` that describes the structure as built:
  - the tree (`Cargo.toml`, every file under `src/` and `tests/`)
  - each module's single contract and the spec it implements
  - the dependency direction (verbs build on `store` and `note`, never on each other)
  - where a new verb goes (`src/<verb>.rs`, its dispatch in `main.rs`, `tests/<verb>.rs` and its own capability spec)
  - unit tests inside modules, CLI behavior in `tests/` through the built binary
  - the `nix shell` line that builds and tests

  Verify that every source file is named and nothing is left over with `rg -q '^## Architecture' AGENTS.md && ! (for f in src/*.rs tests/*.rs tests/common/*.rs; do rg -qF "$f" AGENTS.md || echo "$f"; done | grep .)`

## 7. Integration

- [ ] 7.1 Smoke test the built binary end to end in a fresh store: `new` for two kinds, a refused duplicate topic, then a clean `check`. Verify with `cargo build && export BILBO_HOME="$(mktemp -d)/store" && ./target/debug/bilbo new decision smoke-test && ./target/debug/bilbo new plan other-topic && ! ./target/debug/bilbo new plan smoke-test && ./target/debug/bilbo check`
- [ ] 7.2 Run the full suite: formatting, lints and every test. Verify with `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
