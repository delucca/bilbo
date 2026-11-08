# Testing bilbo

Place tests in the lowest folder containing everything exercised: beside one component; top-level `tests/` only across components. Keep `apps/{cli,serve,macos}` thin; composition lives there, component behavior in `libs/`. Rust and Swift libraries never depend on each other; cross-language suites compose them under `tests/`.

Built so far: `.bazelrc`; `bilbo_test` for Swift in `tools/bazel/swift.bzl` and `bilbo_rust_test` for Rust in `tools/bazel/rust.bzl`, both reading the layers from `tools/bazel/layers.bzl`; the macOS launch test in `tests/e2e/macos/`; the CLI's unit tests (`libs/rust/store`, `apps/cli`), snapshot tests (`apps/cli/tests/`) and e2e tests (`tests/e2e/cli/`). Add other layers as they get something to test.

## Choose a layer

Each layer covers both languages; its name is its Bazel tag. Below, R = `libs/rust/<crate>/`, S = `libs/swift/<Module>/`.

| Layer | Rust boundary | Swift boundary | Folder |
|---|---|---|---|
| unit | Isolated logic, fakes; no disk/database | Isolated logic, fakes; no disk/AppKit/app | R `src/**`, inline `#[cfg(test)]`; S `Tests/Unit/` |
| contract | Protocol encoding, decoding, validation | Same examples; agreement with Rust | `tests/contract/`; golden requests/responses: `libs/proto/examples/` |
| acceptance | Feature behavior through public API | Feature behavior through API/state models | R `tests/`; S `Tests/Acceptance/` |
| integration | Component with real files/SQLite | Component with real infrastructure/AppKit | R `tests/`; S `Tests/Integration/` |
| snapshot | Rendered text/CLI formatting | Recorded state/rendered views | R `tests/`, baselines alongside; S `Tests/Snapshots/` |
| e2e | Real CLI/serve outer interfaces | Real `.app`, including serve connection | `tests/e2e/{cli,serve,macos}/` |
| release | Shipped CLI executable/runtime dependencies; serve as distributed | Signed, notarized bundle: contents, embedded executables, signing/notarization | `tests/release/` |

Acceptance provides primary feature coverage with fakes: no real disk, AppKit or running products. Integration uses real dependencies without driving complete products. Choose by boundary: baseline comparisons of CLI invocations remain e2e; formatting-library snapshots stay beside the library. Release checks packaging independently; test serve inside the bundle if shipped there, without inventing standalone distribution.

## Declare and write tests

- Each layer folder gets a Bazel target. Rust non-unit component layers share `tests/` with separate targets/source sets.
- Declare through `bilbo_test(layer = "acceptance")` from `tools/bazel/swift.bzl` for Swift and `bilbo_rust_test(layer = ...)` from `tools/bazel/rust.bzl` for Rust; each owns tags and size, the Swift one also `copts`. `desktop = True` marks a screen test and exists only on the Swift macro; Rust has no screen tests yet. A Rust unit target passes `crate = ":lib"`; snapshot and e2e targets pass `srcs`.
- Name acceptance tests by feature, with parameterized example tables; no spec IDs.
- Swift uses Swift Testing and `@testable import`; reusable port fakes: S `Testing/`, test-only visibility.
- Assert exact layout numerically in integration; inspect Metal terminal buffer state instead of pixels.
- Compare snapshots with approved baselines. Prefer text, including substantial Rust output deserving review; few pixel snapshots, covering light/dark. Rust snapshots are plain files under the crate's `tests/snapshots/`, listed in the target's `compile_data` and compared with `include_str!`; no snapshot library.
- Screen tests additionally carry `desktop`, `local`, `exclusive`. `desktop` is not a layer. Keep them short: they share user focus/keyboard. `exclusive` serializes them within one Bazel run only; do not run desktop tests from two worktrees at once.

## Exercise real products

Keep e2e to a few smoke flows:

- **CLI:** real `bilbo`, isolated temporary directory; assert stdout, exit code, files, relevant stderr.
- **Serve:** real binary, isolated state; stdio JSON-RPC responses and process behavior.
- **macOS:** real `.app` through Accessibility; Swift Testing `swift_test` targets. Only these use pinned, signed AXorcist `axorc`, through one wrapper in `tests/e2e/macos/`. Harness owns launch/waits/assertions; driver needs Accessibility permission.

CLI/serve e2e need no desktop.

## Run tests

Run `nix develop -c bazel test //...`, optionally adding `--config=<name>`; scope components by replacing `//...`, e.g. `//libs/swift/Notebook/...`. Each configuration replaces the default filter, including desktop tests within a selected layer. `--config=desktop` selects screen tests; `--config=all` selects everything.

```text
test --test_tag_filters=-desktop,-release
test:unit --test_tag_filters=unit
test:contract --test_tag_filters=contract
test:acceptance --test_tag_filters=acceptance
test:integration --test_tag_filters=integration
test:snapshot --test_tag_filters=snapshot
test:e2e --test_tag_filters=e2e
test:release --test_tag_filters=release
test:desktop --test_tag_filters=desktop
test:all --test_tag_filters=
```

## Why

- Local placement keeps component ownership clear.
- Tags select layers across languages; screen access stays opt-in, headless e2e included by default.
- Snapshots only warn of change: review differences; establish correctness through behavioral/numeric assertions.

