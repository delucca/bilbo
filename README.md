# bilbo

Durable memory for coding agents: notes they write and recall, and a library
of sources they cite. The `bilbo` command creates, checks, searches and
indexes those notes.

## Install

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/delucca/bilbo/releases/latest/download/bilbo-installer.sh | sh
```

The binary goes to `$XDG_BIN_HOME` when it is set, else `~/.local/bin`. It
never goes to `~/.cargo/bin`. Run the installer again to upgrade.

The installer writes `~/.config/bilbo/bilbo-receipt.json`. If the binary's
folder is not on your `PATH`, it also writes `env.sh` and `env.fish` next to
the receipt, creates or edits your shell rc files to source `env.sh`, and
writes `~/.config/fish/conf.d/bilbo.env.fish`. Set `BILBO_NO_MODIFY_PATH=1` to
stop the PATH edits.

Release binaries cover macOS (arm64 and Intel) and Linux (x86_64 and arm64,
glibc). On NixOS, use the flake.

### Nix

Try it without installing:

```sh
nix run github:delucca/bilbo -- --version
```

Or add it as a flake input and use `bilbo.packages.${system}.default`:

```nix
inputs.bilbo = {
  url = "github:delucca/bilbo";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

`inputs.nixpkgs-unstable.follows` is optional: only the dev shell reads that
input.

## Agent plugin

Claude Code:

```sh
claude plugin marketplace add delucca/bilbo
claude plugin install bilbo@bilbo
```

Codex:

```sh
codex plugin marketplace add delucca/bilbo
codex plugin add bilbo@bilbo
```

A later `bilbo setup` will automate this.
