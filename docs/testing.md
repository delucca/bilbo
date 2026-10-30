# Testing bilbo

Tests belong in the lowest folder that contains everything they exercise. Keep single-component tests beside the component; use top-level `tests/` only for suites spanning components.

This document defines the convention. The layer targets, macros and `.bazelrc` configurations are not yet implemented. Add each layer when it has something to test.

## Choose a layer

The layer name is also its Bazel tag.

| Layer | What it checks and may touch | Location |
|---|---|---|
| **unit** | Isolated logic with fakes. No real disk, AppKit or app. | Rust: `libs/rust/<crate>/src/**`, inline `#[cfg(test)]`. Swift: `libs/swift/<Module>/Tests/Unit/`. |
| **contract** | Rust and Swift agree on protocol examples. Uses golden request/response pairs from `libs/proto/examples/`. | `tests/contract/` |
| **acceptance** | A feature through a component's public API or Swift state models, using fake dependencies. No real disk, AppKit or app. This is the main source of feature coverage. | Rust: `libs/rust/<crate>/tests/`. Swift: `libs/swift/<Module>/Tests/Acceptance/`. |
| **integration** | A component against real infrastructure: Rust with real disk, Swift with real AppKit views and numeric layout assertions. Does not drive the running app. | Rust: `libs/rust/<crate>/tests/`. Swift: `libs/swift/<Module>/Tests/Integration/`. |
| **e2e** | A few smoke flows through the real `.app` and `serve`, or the CLI. Desktop flows use the real desktop. | `tests/e2e/` |
| **snapshot** | Recorded state or rendered views compared with an approved baseline. Prefer text state; keep pixel snapshots few, covering light and dark appearances. | `libs/swift/<Module>/Tests/Snapshots/` |
| **release** | The signed, notarized distribution bundle. | `tests/release/` |

Choose by the boundary exercised: protocol agreement is contract; feature behavior with fakes is acceptance; real infrastructure is integration; running products are e2e; distribution artifacts are release. Use unit for isolated logic and snapshot for baseline comparisons.

## Declare and write tests

- Give each layer folder its own Bazel test target, tagged with its layer. Rust unit tests stay inline; acceptance and integration share `tests/` but use separate targets and source sets.
- Declare targets through the layer macro in `tools/bazel/`, conceptually `bilbo_test(layer = "acceptance")`. The macro owns tags, visibility and compiler options; its exact API is to be defined.
- Name acceptance tests by feature and use parameterized example tables. Tests cite no spec IDs.
- Swift tests use `@testable import`. Put reusable port fakes in `libs/swift/<Module>/Testing/`, with test-only visibility.
- Keep `apps/` thin: composition belongs there, component behavior and its tests belong in `libs/`. Rust and Swift libraries never depend on each other; cross-language suites compose them under `tests/`.
- Assert exact layout numerically in integration tests. For Metal-rendered terminal content, inspect buffer state instead of relying on pixel capture.

The desktop e2e driver is **to be confirmed**: first verify XCUITest through rules_apple's `macos_ui_test` under Bazel; use palantir's AX driver if that does not work. Swift Testing execution under Bazel also remains to be verified.

## Run tests

Once the targets and configurations exist:

```sh
# Default: every layer except e2e and release.
nix develop -c bazel test //...

# One layer: unit, contract, acceptance, integration, snapshot, e2e or release.
nix develop -c bazel test //... --config=acceptance

# Every layer, e2e and release included.
nix develop -c bazel test //... --config=all

# One layer of one component.
nix develop -c bazel test //libs/swift/Notebook/... --config=acceptance
```

`.bazelrc` sets `test --test_tag_filters=-e2e,-release` and one `test:<layer> --test_tag_filters=<layer>` per layer, so a `--config` replaces the default rather than adding to it. Desktop e2e runs share focus and keyboard input with the user, so keep them short and serialize access to the desktop.

## Why

- **Beside the component:** ownership stays local. Top-level suites hold only checks that span components.
- **Tags select layers:** a layer spans many component folders. Bazel tags select it across the repository without moving tests or relying on Swift Testing tag filtering.
- **Snapshots warn of change:** a matching baseline does not prove the UI is correct. Review differences and use behavioral or numeric assertions to establish correctness.

