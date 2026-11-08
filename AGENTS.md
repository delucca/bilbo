# bilbo

## Testing

- Read `docs/testing.md` before adding or moving a test.
- `nix develop -c bazel test //...` runs every layer except tests that need the screen (`desktop`) and release.
- `--config=<layer>` runs one layer (`unit`, `contract`, `acceptance`, `integration`, `snapshot`, `e2e`, `release`); `--config=desktop` runs only screen tests; `--config=all` runs everything. Desktop tests take focus and keyboard input. A layer config also runs that layer's desktop tests: add `--test_tag_filters=<layer>,-desktop` to stay headless.

## Rust

- After changing `Cargo.toml` or `Cargo.lock`, repin with `CARGO_BAZEL_REPIN=1 nix develop -c bazel build @crates//:all`, then run a plain `nix develop -c bazel build //apps/cli` before committing: the repin records `CARGO_BAZEL_REPIN` in `MODULE.bazel.lock`, and only a plain build reverts it.
- Lint with `nix develop -c bazel build //... --config=lint` (clippy with `-D warnings`, and rustfmt; format with `nix develop -c bazel run @rules_rust//:rustfmt`). Every Rust target sets `lint_config` to the `extract_cargo_lints` target of its Cargo package; without it `[workspace.lints]` silently does not apply.
- The dev shell has no `cargo`; run it as `nix develop -c bazel run @rules_rust//tools/upstream_wrapper:cargo -- <args> --manifest-path "$PWD/Cargo.toml"`. `cargo test --locked` and `cargo clippy --locked --all-targets -- -D warnings` must pass too: nextest, cargo-mutants and cargo-llvm-cov run through Cargo.
- Keep `BILBO_VERSION` in `tools/bazel/version.bzl` equal to `workspace.package.version` in `Cargo.toml`, and give every target that reads `CARGO_PKG_VERSION` `version = BILBO_VERSION`; nothing checks either.
- A verb's CLI side is the folder `apps/cli/src/<verb>/` (`mod.rs`: its clap `Args` with its page, and `run`; `view.rs`: its human view), registered as one `Verb` variant and one arm in `apps/cli/src/lib.rs`. Its logic lives in the lib crate of its domain under `libs/rust/`, never in a crate of all verbs; a new crate lists exactly its dependents in `visibility`. String options use `NonEmptyStringValueParser`, so `--kind=` fails with `--kind needs a value`.
- Snapshot baselines in `apps/cli/tests/snapshots/` are plain files: after reading a failing diff, copy the rendered text over the baseline.
