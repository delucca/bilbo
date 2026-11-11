# Testing bilbo

Place tests in the lowest folder containing everything exercised: beside one component; top-level `tests/` only across components. Keep `apps/{cli,serve,macos}` thin; composition lives there, component behavior in `libs/`. Rust and Swift libraries never depend on each other; cross-language suites compose them under `tests/`.

Built so far: `.bazelrc`; `bilbo_test` for Swift in `tools/bazel/swift.bzl` and `bilbo_rust_test` for Rust in `tools/bazel/rust.bzl`, both reading the layers from `tools/bazel/layers.bzl`; the macOS launch test in `tests/e2e/macos/`; the CLI's unit tests (`libs/rust/settings`, `libs/rust/store`, `libs/rust/kinds`, `libs/rust/kinds/markdown`, `libs/rust/kinds/session` on its synthetic fixtures, `apps/cli`, whose unit target also checks `docs/reference/configuration.md` against the key registry), the contract target of the session kind's Claude dialect (`libs/rust/kinds/session/tests/contract/`: its passages against what the eval's `build.py` printed for the same lines, with an ignored test for the whole eval corpus; `expected.jsonl` there is written by `regenerate.py --build <the eval's build.py>`, a manual tool, and never edited by hand), the acceptance target of the `remember`, follows and `read` use cases (`libs/rust/kinds/tests/`), integration tests (`libs/rust/store/tests/`, the again flag's races among them, `libs/rust/settings/tests/` over the config file on a real disk, `apps/cli/tests/remember.rs`, `apps/cli/tests/sessions.rs`, `apps/cli/tests/read.rs` (every fixture remembered and read back a page at a time, and an ignored test that does the same over a scratch store the done check fills), `apps/cli/tests/timer.rs` over the unit files in `docs/reference/timer/`, and `apps/cli/tests/readback.rs` and the real-folder test of `apps/cli/tests/sessions.rs`, ignored by default: the done checks run them against a scratch store), the targets of `libs/rust/search` (unit: the chunker with a fake tokenizer, the analyzer, the position's encoding; acceptance: the `reindex` use case with fake `Entries`, `Reader`, `Tokens` and `Index`; contract: its chunks and terms against what the retrieval eval's bm25 store holds and bm25s gives, with ignored tests for the whole store), of `libs/rust/search/qwen` (unit, and contract: its token counts against the same store's) and of `libs/rust/search/tantivy` (unit, and integration: an index on a real disk), `apps/cli/tests/reindex.rs` (integration: `remember` then `reindex` in one process with the real kinds, tokenizer and index, and an ignored test that prints what the index of a done-check store holds), snapshot tests (`apps/cli/tests/`), the layout test of `apps/cli/src` (`apps/cli/tests/layout.rs`, layer integration) and e2e tests (`tests/e2e/cli/`). Add other layers as they get something to test.

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

The CLI e2e target and `//libs/rust/store:unit` are `size = "medium"` whatever their layer says, both sized for a full parallel run (the store's unit tests take 8 s alone and up to 40 s beside the rest). The e2e target runs the real binary, which loads the tokenizer in every indexing run and cannot share one, so it takes about 20 s alone and over 50 s beside the rest of the suite, too near the 60 s of `small`. `apps/cli/tests/reindex.rs` loads one tokenizer for the whole binary and asks for two workers, for the same reason.

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

