# bilbo

## Testing

- Read `docs/testing.md` before adding or moving a test.
- `nix develop -c bazel test //...` runs every layer except tests that need the screen (`desktop`) and release.
- `--config=<layer>` runs one layer (`unit`, `contract`, `acceptance`, `integration`, `snapshot`, `e2e`, `release`); `--config=desktop` runs only screen tests; `--config=all` runs everything. Desktop tests take focus and keyboard input.

## Rust

- `nix develop -c bazel run //apps/cli -- --help` runs the CLI; the binary is `bazel-bin/apps/cli/bilbo`.
- Dependencies live in the root `Cargo.toml` (caret versions, never `*` or `=`) and `Cargo.lock`. After changing either, repin with `CARGO_BAZEL_REPIN=1 nix develop -c bazel build @crates//:all`; Bazel rewrites `Cargo.lock` and `MODULE.bazel.lock`. The repin records `CARGO_BAZEL_REPIN` in `MODULE.bazel.lock` and the next plain build reverts it, so run a plain `nix develop -c bazel build //apps/cli` before committing the two locks. A build that uses no crate does not rewrite the lock.
- Lint: `nix develop -c bazel build //... --config=lint` runs clippy with `-D warnings` and the rustfmt check over every Rust target, tests included. Format with `nix develop -c bazel run @rules_rust//:rustfmt`. The lint table is `[workspace.lints]` in `Cargo.toml`. Every Rust target sets `lint_config` (`:lints`, an `extract_cargo_lints` of its package), tests included, or the table does not apply to it; a new Rust target must set it too; `//tests/e2e/cli:cli` has no Cargo package of its own and reuses `//apps/cli:lints`.
- Cargo with the Bazel toolchain: `nix develop -c bazel run @rules_rust//tools/upstream_wrapper:cargo -- build --locked --manifest-path "$PWD/Cargo.toml"`. It writes `/target` (ignored) unless `CARGO_TARGET_DIR` is set. `cargo test --locked` and `cargo clippy --locked --all-targets -- -D warnings` work the same way on the whole workspace: the end-to-end tests are a `[[test]]` of `apps/cli` that finds the binary through `CARGO_BIN_EXE_bilbo` under Cargo and the runfiles under Bazel.
- `BILBO_VERSION` in `tools/bazel/version.bzl` and `workspace.package.version` in `Cargo.toml` must match by hand. A target that reads `CARGO_PKG_VERSION` sets `version = BILBO_VERSION`.
- `apps/cli/src/main.rs` is the only writer of stdout and stderr; `bilbo_cli` returns an `Outcome` (stdout per view, diagnostics, exit 0 or 1) or a `Failure`, and `route::plan` decides what goes to which stream and in what order (diagnostics follow the result only when stdout gets the human view). Every form of help renders through `help::overview` and `help::page`; a verb is a variant of `cli::Verb` (clap derive, string options with `NonEmptyStringValueParser` so `--kind=` is `--kind needs a value`; `cli::command()` gives each the page shape of `help::verb`, with its usage forms joined by `\n  ` and its help text wrapped by hand to 80 columns), and the page guards in `help.rs` cover every registered verb. A snapshot test prints both texts when it fails; after reading the diff, copy the rendered text over the baseline in `apps/cli/tests/snapshots/`.
