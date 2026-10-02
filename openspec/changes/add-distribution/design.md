# Design

## Context

- **No releases yet.** bilbo is one crate at `0.1.0`, with `publish = false`, a committed `Cargo.lock` and `rust-version = "1.95"`.
- **One workflow today.** `.github/workflows/ci.yml` is the only workflow: one `verify` job on Rust 1.95.0, with every action pinned by commit SHA. The `main pull requests` ruleset requires that job.
- **The binary embeds no paths.** It reads the store, config and cache folders from the environment at run time, so any prebuilt build works on any machine.
- **dnix already consumes your repos as flake inputs.**
  - dmacs: `packages.<system>.dmacs`, pulled in through an overlay.
  - trademate: `packages` plus `nixosModules.default`.
  - Both are inputs of the form `git+https://github.com/delucca/<repo>.git?shallow=1` with `inputs.nixpkgs.follows = "nixpkgs"`.
  - dnix's `nixpkgs` is `nixpkgs-26.05-darwin`, which ships rustc 1.95.0 and cargo-dist 0.31.0. `nixpkgs-unstable` ships cargo-dist 0.33.0, the current release.
- **The plugin follows `main`.** Claude Code installs whatever `main` holds, because the plugin has no `version` (`agent-plugin` spec). A pinned binary and an unpinned skill can therefore drift apart. This change only makes the plugin files travel with the binary. add-setup decides how each tool installs them.

## Goals / Non-Goals

**Goals:**
- **One source for the version number:** `Cargo.toml`. The tag, the binary, the Nix package and the Codex manifest all match it, and a test or the release plan rejects a mismatch.
- **Nothing hand-written in the release pipeline that dist can generate.** Our own code stays limited to `--version` and the flake.
- **The release workflow meets the same pinning rule as `ci.yml`.**

**Non-Goals:**
- **Cross-compiling in the flake.** Each system builds natively. The flake is how Nix users get bilbo, not how releases are built.
- **Caching Nix builds** in a binary cache.
- **Changing `ci.yml`'s `verify` job.**

## Decisions

### dist generates the release workflow and the installer

`dist init` writes `dist-workspace.toml` and `.github/workflows/release.yml`. From then on, `dist generate` rewrites the workflow from the config, and nobody edits the YAML by hand. On a tag push the workflow:

1. runs `dist plan`, which fails when the tag matches no package version;
2. builds each target on a native GitHub runner;
3. uploads the archives, their `.sha256` files and `bilbo-installer.sh` to a release named for the tag.

On a pull request it runs `dist plan` only, which is dist's default `pr-run-mode`. A broken dist config therefore shows up before a tag is pushed.

- **Why not a hand-written `install.sh` and `release.yml`.** We would own platform detection, archive naming, checksum files, PATH editing and the build matrix. dist's installer already handles all of that and is used by uv, ruff and cargo-dist itself. Our script would need its own tests across shells and OSes.
- **Why not `cargo install --git`.** Every user would need Rust 1.95 or newer. The people bilbo targets run Claude Code or Codex, not necessarily a Rust toolchain.
- **Why `dist-workspace.toml` rather than `[workspace.metadata.dist]` in `Cargo.toml`.** It is where current `dist init` writes. It also keeps `Cargo.toml` to what Cargo reads, apart from `[profile.dist]`.

`publish = false` stays, since bilbo is not on crates.io. dist treats `publish = false` as "don't distribute", so `dist-workspace.toml` sets `dist = true` to override that.

### The dist config

- `cargo-dist-version = "0.33.0"`. CI installs exactly this version. The dev shell gets the same one from a pinned `nixpkgs-unstable` input.
- `ci = "github"` and `installers = ["shell"]`. No PowerShell, no Homebrew and no npm installer. Each would add a platform or a publishing secret that nobody has asked for.
- `targets = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]`.
- `install-path = ["$XDG_BIN_HOME/", "~/.local/bin/"]`.
  - dist's default is `~/.cargo/bin`, which makes no sense for people who don't use Rust.
  - The cascade mirrors how bilbo already resolves `XDG_DATA_HOME`, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`.
  - `~/.local/bin` is also where uv and pipx install, so it is usually on PATH already.
- `install-updater = false`. Upgrading means running the installer again. A separate `bilbo-update` binary is one more thing to explain.
- `[dist.github-action-commits]` pins every action the generated workflow uses (checkout, upload and download artifact, and the others `dist init` lists) to the commit SHA of the tag dist expects.
  - `tests/workflows.rs` reads every file in `.github/workflows/` and fails on any `uses:` line whose ref is not a 40-hex SHA.
  - That catches an action dist adds in a later version that we forgot to pin, and it covers `ci.yml` too.

### glibc builds, not musl

`ureq` uses rustls, and bilbo links no C library, so a `*-linux-musl` static build would work and would run on any distro. dist builds the gnu targets on its default Ubuntu runner. That sets a glibc floor, about 2.35 on Ubuntu 22.04, which every distro supported in 2026 meets. Switching to musl later only changes `targets`, and nobody has reported a need for it.

### `--version` is a top-level argument only

`main` checks for `args == ["--version"]` before dispatch and prints `bilbo {CARGO_PKG_VERSION}`.

- **No `-V`.** Every short flag becomes something a verb can no longer use.
- **`--version` after a verb stays an unknown option**, the same way an unknown option already behaves, so verbs keep their own option space.
- **`bilbo --version now` falls through to dispatch.** There `--version` is an unknown verb, which gives the usage error in the spec without any special case.

`--help` stays as it is: it is accepted anywhere before `--`.

### The flake

```
inputs:
  nixpkgs          github:NixOS/nixpkgs/nixpkgs-26.05-darwin
  nixpkgs-unstable github:NixOS/nixpkgs/nixpkgs-unstable   (dev shell only: dist 0.33.0)
outputs, for aarch64-darwin, x86_64-linux, aarch64-linux:
  packages.default = packages.bilbo
  checks.default   = packages.bilbo
  devShells.default
```

- **Systems.** These match dmacs's three. Intel Macs use the shell installer. nixpkgs keeps x86_64-darwin builds only for a limited time, and dnix has no Intel host.
- **The package uses `rustPlatform.buildRustPackage`:**
  - `cargoLock.lockFile = ./Cargo.lock` gives a vendor hash that never needs updating;
  - `version` comes from `(lib.importTOML ./Cargo.toml).package.version`;
  - `meta.mainProgram = "bilbo"`.
- **`src` is a `lib.fileset`** of `Cargo.toml`, `Cargo.lock`, `src`, `tests`, `plugins` and the two marketplace folders.
  - Edits to `openspec/`, `README.md` or the workflows don't rebuild the package.
  - `tests/plugin.rs` reads the marketplaces, which is why the build needs them.
- **Tests run in `checkPhase`.**
  - The fake embedder listens on `127.0.0.1`. The Linux sandbox has loopback. On macOS, `__darwinAllowLocalNetworking = true` allows it when the sandbox is on.
  - The ignored speed test stays ignored.
- **`postInstall` copies the marketplace layout** into `$out/share/bilbo/`: `plugins/bilbo`, `.claude-plugin/marketplace.json` and `.agents/plugins/marketplace.json`.
  - A marketplace's `source: "./plugins/bilbo"` resolves relative to its root, so the folder is a marketplace either tool can add by path.
  - add-setup builds on this. A `claude plugin marketplace add <store path>` installs the skills from the binary's own commit.
- **The dev shell** provides `cargo`, `rustc`, `clippy` and `rustfmt` from `nixpkgs`, and `cargo-dist` from `nixpkgs-unstable`.
  - In `AGENTS.md`, `nix develop -c <cmd>` replaces the four-package `nix shell` line, and `CARGO_TARGET_DIR` still points at the checkout's `target`.
  - `nixpkgs-unstable` is only evaluated for the dev shell. A consumer that sets `inputs.nixpkgs-unstable.follows` reuses its own copy, and one that doesn't never fetches it to build the package.
- **Why the flake lives in this repo, not in dnix.** The plan already chose this. A copy in dnix would drift from `Cargo.lock` and the plugin, and other Nix users get bilbo with no dnix involved.

### A Nix job in CI

`ci.yml` gains a `nix` job on `ubuntu-latest`:

- it installs Nix with `cachix/install-nix-action`, pinned by SHA;
- it runs `nix flake check -L`, which builds the x86_64-linux package and runs its tests;
- it also runs `nix eval .#packages.aarch64-darwin.default.version`, so a darwin-only evaluation error fails too.

The job is not added to the required checks without the maintainer's go-ahead, because the ruleset is a repository setting.

## Risks / Trade-offs

- **Unverified checksums.** dist's shell installer downloads over HTTPS but does not check the `.sha256` yet. → The proposal records this as a non-goal. Users who want verification download the archive and its checksum by hand.
- **Release workflow drift.** A dist upgrade can add actions or change the workflow. → `tests/workflows.rs` fails on an unpinned action. Upgrading dist is a deliberate edit to `cargo-dist-version` followed by `dist generate`, and its diff is reviewed like code.
- **Nix and CI toolchains can differ.** nixpkgs 26.05 may move past rustc 1.95.0 in a point update while CI stays on 1.95.0. → That only matters if the newer compiler rejects the code, which `nix flake check` in CI would catch. `rust-version` stays the floor.
- **The plugin can still drift for curl users.** The installer ships only the binary, and the Claude Code plugin still follows `main`. → add-setup solves this. Codex takes `--ref v<version>`, and Claude Code can add a marketplace from a path or a pinned source. This change does not make it worse.
- **The installer edits shell rc files** when the install folder is not on PATH. → That is dist's documented behavior, and the installer prints what it changed. `README.md` mentions it.

## Migration Plan

1. Merge this change. CI runs `dist plan` and the Nix job.
2. Bump `Cargo.toml` and `.codex-plugin/plugin.json` to `0.1.0` if they still say so, then push the tag `v0.1.0`. Check the release assets and run the installer once on macOS arm64 and once on Linux.
3. dnix: add the `bilbo` input, `follows` both nixpkgs inputs, and add the package to rivendell's packages. That dnix commit happens after this change merges and is not part of the change.

Rollback: delete the release and the tag. Nothing else depends on them until add-setup.
