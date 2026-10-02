# Tasks

Until group 2 lands the dev shell, run cargo commands inside `nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#clippy nixpkgs#rustfmt -c <cmd>`, and after it inside `nix develop -c <cmd>`. Run everything from the repo root with `CARGO_TARGET_DIR` set to the checkout's `target`.

## 1. `bilbo --version` (`cli` delta)

- [ ] 1.1 In `src/main.rs`, print `bilbo {CARGO_PKG_VERSION}` to stdout and exit 0 when the arguments are exactly `["--version"]`, before dispatch. Add `bilbo --version` to the USAGE text. Cover the three `Version` scenarios in `tests/cli.rs`: stdout matches `env!("CARGO_PKG_VERSION")`, `check --version` is an unknown option, and `--version now` is a usage error. Verify with `cargo test --locked --test cli`

## 2. Nix flake (`distribution`: Nix package, The package carries the plugin)

- [ ] 2.1 Pull the current docs for `rustPlatform.buildRustPackage` (`cargoLock.lockFile`, `checkPhase`, `__darwinAllowLocalNetworking`) and `lib.fileset` with `use-context7`. Add `flake.nix` as design.md lays it out:
  - inputs `nixpkgs` (`nixpkgs-26.05-darwin`) and `nixpkgs-unstable`;
  - `packages.default`, `checks.default` and `devShells.default` for `aarch64-darwin`, `x86_64-linux` and `aarch64-linux`;
  - `src` built as a `lib.fileset`;
  - the version read from `Cargo.toml`;
  - `postInstall` copying the marketplace layout into `$out/share/bilbo/`.

  Commit `flake.lock`. Verify with `nix build -L && ./result/bin/bilbo --version && test -f result/share/bilbo/plugins/bilbo/skills/recall/SKILL.md && test -f result/share/bilbo/.claude-plugin/marketplace.json && test -f result/share/bilbo/.agents/plugins/marketplace.json`
- [ ] 2.2 Check the plugin scenarios against the built package. Verify with `claude plugin validate result/share/bilbo && diff -r plugins/bilbo result/share/bilbo/plugins/bilbo`
- [ ] 2.3 Check that a failing test fails the build. Break one assertion in a scratch copy, outside the checkout, and confirm `nix build` fails. Record the command in the commit message, not in the repo. Verify with `nix build -L` succeeding again on the unmodified checkout
- [ ] 2.4 Update `AGENTS.md`:
  - replace the `nix shell nixpkgs#cargo …` instruction with `nix develop -c <cmd>`;
  - add `flake.nix` and `flake.lock` to Architecture;
  - say that `share/bilbo/` in the package is a local marketplace.

  Verify with `nix develop -c cargo --version && rg -q 'nix develop' AGENTS.md && ! rg -q 'nix shell nixpkgs#cargo' AGENTS.md`

## 3. Release pipeline (`distribution`: Releases come from version tags, Release assets, Shell installer)

- [ ] 3.1 Add `cargo-dist` to the dev shell from `nixpkgs-unstable`. Pull the current dist docs with `use-context7` for `dist init`, `dist-workspace.toml`, `install-path`, `github-action-commits` and `pr-run-mode`. Then run `dist init` with:
  - CI `github`;
  - installer `shell`;
  - the four targets from design.md.

  Then set `dist = true`, `install-path = ["$XDG_BIN_HOME/", "~/.local/bin/"]` and `install-updater = false`. Verify with `nix develop -c dist --version | rg -q '0\.33\.0' && test -f dist-workspace.toml && test -f .github/workflows/release.yml`
- [ ] 3.2 For every action in the generated `release.yml`, look up the commit SHA of the tag dist uses with `gh api repos/<owner>/<repo>/git/ref/tags/<tag>`, dereferencing annotated tags. Add each to `[dist.github-action-commits]`, then run `dist generate`. Verify with `nix develop -c dist generate --check` (or the 0.33.0 equivalent named in its docs) `&& ! rg -n 'uses: [^ ]+@v[0-9]' .github/workflows/release.yml`
- [ ] 3.3 Add `tests/workflows.rs`. It reads every file in `.github/workflows/` and fails on any `uses:` whose ref is not 40 lowercase hex characters, naming the file and line. Verify with `cargo test --locked --test workflows`
- [ ] 3.4 Check the plan locally for the current version and for a mismatched tag. Verify with `nix develop -c dist plan --tag v$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')` succeeding, listing four archives and `bilbo-installer.sh`, and with `nix develop -c dist plan --tag v9.9.9` failing
- [ ] 3.5 Update `AGENTS.md`:
  - list `dist-workspace.toml`, `release.yml` (generated, never edited by hand) and `tests/workflows.rs`;
  - record the release steps: bump `Cargo.toml` and `.codex-plugin/plugin.json` together, merge, tag `v<version>`, push the tag;
  - record how to upgrade dist: change `cargo-dist-version` and the dev-shell input together, run `dist generate`, re-pin.

  Verify with `rg -q 'dist-workspace.toml' AGENTS.md && rg -q 'workflows.rs' AGENTS.md`

## 4. CI Nix job

- [ ] 4.1 Add a `nix` job to `.github/workflows/ci.yml`:
  - `cachix/install-nix-action` pinned by SHA;
  - `nix flake check -L`;
  - `nix eval .#packages.aarch64-darwin.default.version`.

  `tests/workflows.rs` covers the new pin. Verify with `cargo test --locked --test workflows` and by pushing the branch and seeing the `nix` job pass in `gh pr checks`
- [ ] 4.2 Ask the maintainer whether to add `nix` to the required checks of the `main pull requests` ruleset (id 24376306). Change the ruleset only on a yes. Verify with `gh api repos/delucca/bilbo/rulesets/24376306 --jq '.rules[] | select(.type=="required_status_checks")'` showing the agreed list

## 5. README

- [ ] 5.1 Write `README.md` with the `write-readme` skill. It covers:
  - what bilbo is, in two sentences;
  - the curl install line, including where the binary lands and that the installer may edit the shell rc;
  - the Nix route: `nix run github:delucca/bilbo` and the flake input with `follows`;
  - installing the plugin from the marketplaces as the `agent-plugin` spec describes, until add-setup automates it.

  Verify with `rg -q 'bilbo-installer.sh' README.md && rg -q 'nix run github:delucca/bilbo' README.md`

## 6. Integration

- [ ] 6.1 Run the full suite. Verify with `nix develop -c sh -c 'cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked' && nix flake check -L`
- [ ] 6.2 After merge, with the maintainer's go-ahead, push the tag `v<version>` and check the release. Record the commands and output in the change folder as `smoke.md`:
  - it holds four archives, four `.sha256` files and `bilbo-installer.sh`;
  - every checksum matches;
  - the installer, run with a temporary `HOME` on macOS arm64 and on Linux, installs into `$HOME/.local/bin`, and into `$XDG_BIN_HOME` when set, with `--version` printing the tag's version.

  Verify with `test -s openspec/changes/add-distribution/smoke.md`
