# bilbo

## Testing

- Read `docs/testing.md` before adding or moving a test.
- `nix develop -c bazel test //...` runs every layer except tests that need the screen (`desktop`) and release.
- `--config=<layer>` runs one layer (`unit`, `contract`, `acceptance`, `integration`, `snapshot`, `e2e`, `release`); `--config=desktop` runs only screen tests; `--config=all` runs everything. Desktop tests take focus and keyboard input. A layer config also runs that layer's desktop tests: add `--test_tag_filters=<layer>,-desktop` to stay headless.
- Every run of the built binary, by hand or in a test, sets `BILBO_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` to scratch folders and `BILBO_CONFIG` to a scratch `config.toml` that exists, empty or not: `BILBO_HOME` moves only the store, and a missing file that `BILBO_CONFIG` names is a config error.
- Every `remember` test also needs `HOME` set to a scratch folder, or a config that declares fixture follows: a bare run and a target run that finds the again flag run the follows, a hook consults them to pick the transcript's kind and never runs them, and the default follows are the real `~/.claude/projects` and `~/.codex/sessions`.
- Session fixtures in `libs/rust/kinds/session/fixtures/` are synthetic, written in the harness's exact field names and nesting; never copy a line of a real session into the repo.
- `libs/rust/kinds/session/tests/contract/expected.jsonl` is what `build.py` prints, written by `regenerate.py` beside it and never by hand: run it with `--build <the eval's build.py>` after changing `edges.jsonl`, and with `--corpus <unpacked corpus>` to pick `sample.jsonl` again. A line `build.py` stops on cannot go in either file: it belongs in the reader's unit tests.
- A `read` test that needs a page to end or a record to be cut passes `--budget 256`, the smallest: the fixtures fit the default page.
- A test that holds `store.lock` to stand for another writer also holds `store.writer` in the state folder; with `store.lock` alone it stands for a reader, and a bare run or a hook waits for it instead of setting the again flag.

## Rust

- After changing `Cargo.toml` or `Cargo.lock`, repin with `CARGO_BAZEL_REPIN=1 nix develop -c bazel build @crates//:all`, then run a plain `nix develop -c bazel build //apps/cli` before committing: the repin records `CARGO_BAZEL_REPIN` in `MODULE.bazel.lock`, and only a plain build reverts it.
- Lint with `nix develop -c bazel build //... --config=lint` (clippy with `-D warnings`, and rustfmt; format with `nix develop -c bazel run @rules_rust//:rustfmt`). Every Rust target sets `lint_config` to the `extract_cargo_lints` target of its Cargo package; without it `[workspace.lints]` silently does not apply.
- The dev shell has no `cargo`; run it as `nix develop -c bazel run @rules_rust//tools/upstream_wrapper:cargo -- <args> --manifest-path "$PWD/Cargo.toml"`. `cargo test --locked` and `cargo clippy --locked --all-targets -- -D warnings` must pass too: nextest, cargo-mutants and cargo-llvm-cov run through Cargo.
- Keep `BILBO_VERSION` in `tools/bazel/version.bzl` equal to `workspace.package.version` in `Cargo.toml`, and give every target that reads `CARGO_PKG_VERSION` `version = BILBO_VERSION`; nothing checks either.
- A verb's CLI side is the folder `apps/cli/src/<verb>/` (`mod.rs`: its clap `Args` with its page, and `run`; `view.rs`: its human view), registered as one `Verb` variant and one arm in `apps/cli/src/lib.rs`. `kinds()`, `readers()` and `kind_keys()` there are the only code that names a kind crate; a verb gets them as arguments, and a new kind joins the first two and, when it declares a key, the third. Its logic lives in the lib crate of its domain under `libs/rust/`, never in a crate of all verbs; a new crate lists exactly its dependents in `visibility`. String options use `NonEmptyStringValueParser`, so `--kind=` fails with `--kind needs a value`.
- Snapshot baselines in `apps/cli/tests/snapshots/` are plain files: after reading a failing diff, copy the rendered text over the baseline.
- A tunable is a settings key: its default, variable and range live only in its `Key` descriptor, beside the options of the crate that reads it, and `keys()` in `apps/cli/src/console/settings.rs` registers it (a kind crate's key is handed in from `kind_keys()` in `lib.rs`: `keys` and `load` take it as an argument); a clap flag for a key has no `default_value` and no `env`. The folder keys are the composition root's, in `console/settings.rs`, since more than one crate uses the folders.
