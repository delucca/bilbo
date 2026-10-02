# Spec Delta

## Purpose

How the bilbo binary and its agent plugin reach a machine: the release a version tag publishes, the shell installer, and the Nix package, so a user needs neither a Rust toolchain nor a clone.

## ADDED Requirements

### Requirement: Releases come from version tags
Pushing a tag `v<version>` whose `<version>` equals the `version` in `Cargo.toml` at that commit SHALL publish a GitHub Release for the tag. A tag that does not match SHALL publish nothing, and a push without a tag SHALL publish nothing.

#### Scenario: A matching tag publishes a release
- **WHEN** `Cargo.toml` says `version = "0.2.0"` and the maintainer pushes the tag `v0.2.0`
- **THEN** a GitHub Release `v0.2.0` appears on `delucca/bilbo`

#### Scenario: A mismatched tag publishes nothing
- **WHEN** `Cargo.toml` says `version = "0.2.0"` and the maintainer pushes the tag `v0.3.0`
- **THEN** the release workflow fails and no release `v0.3.0` exists

#### Scenario: A push to main publishes nothing
- **WHEN** a commit is pushed to `main` with no tag
- **THEN** no release is created or changed

### Requirement: Release assets
A release SHALL hold one archive with the `bilbo` binary for each of `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, a `<archive>.sha256` file next to each archive, and `bilbo-installer.sh`. The binary in every archive SHALL print the tag's version on `bilbo --version`.

#### Scenario: Every target has an archive and a checksum
- **WHEN** the release `v0.2.0` is published
- **THEN** it holds four archives, one per target, and each archive's SHA-256 equals the hash in its `.sha256` file

#### Scenario: The archived binary knows its version
- **WHEN** the `aarch64-apple-darwin` archive of `v0.2.0` is unpacked and its `bilbo --version` runs
- **THEN** it prints `bilbo 0.2.0`

#### Scenario: No Windows asset
- **WHEN** the release `v0.2.0` is published
- **THEN** it holds no Windows archive and no PowerShell installer

### Requirement: Shell installer
`bilbo-installer.sh` SHALL install the `bilbo` binary of its release for the running OS and CPU into `$XDG_BIN_HOME` when that is set and not empty, otherwise into `$HOME/.local/bin`, creating the folder when needed. Run again from a newer release, it SHALL replace the binary. On an OS or CPU with no archive it SHALL exit non-zero and install nothing.

#### Scenario: The default location
- **WHEN** `XDG_BIN_HOME` is unset, `HOME` is `/Users/a` and the user runs the installer of `v0.2.0` on macOS arm64
- **THEN** `/Users/a/.local/bin/bilbo` exists and its `--version` prints `bilbo 0.2.0`

#### Scenario: XDG_BIN_HOME wins
- **WHEN** `XDG_BIN_HOME` is `/Users/a/bin` and the user runs the installer
- **THEN** `/Users/a/bin/bilbo` exists and nothing is written to `/Users/a/.local/bin` or `/Users/a/.cargo/bin`

#### Scenario: A rerun upgrades
- **WHEN** `bilbo 0.2.0` is installed and the user runs the installer of `v0.3.0`
- **THEN** the same path holds a binary whose `--version` prints `bilbo 0.3.0`

#### Scenario: An unsupported platform
- **WHEN** the user runs the installer on FreeBSD
- **THEN** it prints an error naming the platform, exits non-zero, and creates no `bilbo` binary

### Requirement: Nix package
The repository SHALL be a Nix flake whose `packages.<system>.default`, for `aarch64-darwin`, `x86_64-linux` and `aarch64-linux`, builds `bin/bilbo` from the committed `Cargo.lock`, with the package version taken from `Cargo.toml`. The build SHALL run the test suite, and a failing test SHALL fail the build.

#### Scenario: nix run works
- **WHEN** a user with Nix runs `nix run github:delucca/bilbo/v0.2.0 -- --version`
- **THEN** it prints `bilbo 0.2.0`

#### Scenario: A failing test fails the build
- **WHEN** a commit breaks a test and someone runs `nix build` on it
- **THEN** the build fails and produces no `bilbo` package

### Requirement: The package carries the plugin
The Nix package SHALL hold, under `share/bilbo/`, the plugin folder `plugins/bilbo/` and both marketplaces, `.claude-plugin/marketplace.json` and `.agents/plugins/marketplace.json`, identical to the repository at the commit it was built from. Claude Code and Codex SHALL accept that folder as a local marketplace, so a machine can install the plugin from the same commit as its binary.

#### Scenario: The folder is a valid marketplace
- **WHEN** `nix build` finishes and someone runs `claude plugin validate result/share/bilbo`
- **THEN** the marketplace validates, with at most the missing-version warning the repository's own marketplace also gets

#### Scenario: A later commit does not change an installed package
- **WHEN** a machine's lock pins commit A and `main` later changes `plugins/bilbo/skills/recall/SKILL.md`
- **THEN** the package of commit A still holds the skill as it was at commit A
