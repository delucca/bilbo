# Install

This page shows how to install, upgrade and remove bilbo, with the installer
script, with Nix or with home-manager. After you install, run [`bilbo
setup`](guides/setup.md).

## Install with the installer

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/delucca/bilbo/releases/latest/download/bilbo-installer.sh | sh
```

The binary goes to `$XDG_BIN_HOME` when it is set, else `~/.local/bin`. It never
goes to `~/.cargo/bin`.

Release binaries cover macOS (arm64 and Intel) and Linux (x86_64 and arm64,
glibc). On NixOS, use the flake.

### What the installer writes

- The binary, in the folder above.
- `~/.config/bilbo/bilbo-receipt.json`.
- When the binary's folder is not on your `PATH`: `env.sh` and `env.fish` next
  to the receipt, your shell rc files created or edited to source `env.sh`, and
  `~/.config/fish/conf.d/bilbo.env.fish`.

Set `BILBO_NO_MODIFY_PATH=1` to stop the PATH edits:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/delucca/bilbo/releases/latest/download/bilbo-installer.sh | BILBO_NO_MODIFY_PATH=1 sh
```

### Upgrade

Run the installer again. It replaces the binary. Then run `bilbo setup` again:
the plugin is installed at the binary's own version, so setup brings it up to
date, and with the same inputs every other step is `kept`.

## Install with Nix

Try it without installing:

```sh
nix run github:delucca/bilbo -- --version
```

The flake builds for `aarch64-darwin`, `x86_64-linux` and `aarch64-linux`. On an
Intel Mac, use the installer.

To install it, add the flake as an input and use
`bilbo.packages.${system}.default`:

```nix
inputs.bilbo = {
  url = "github:delucca/bilbo";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

`inputs.nixpkgs-unstable.follows` is optional: only the dev shell reads that
input.

## Install with home-manager

The flake has a home-manager module. It writes the config from `settings` and
runs `bilbo setup --yes` on activation. Add the input with home-manager
following yours:

```nix
inputs.bilbo = {
  url = "github:delucca/bilbo";
  inputs.nixpkgs.follows = "nixpkgs";
  inputs.home-manager.follows = "home-manager";
};
```

Then enable the module:

```nix
{ inputs, ... }:
{
  imports = [ inputs.bilbo.homeManagerModules.default ];

  programs.bilbo = {
    enable = true;
    storeRoot = "/Users/me/notes"; # default: $XDG_DATA_HOME/bilbo
    settings = {
      "embedder.url" = "http://localhost:11434";
      "embedder.model" = "nomic-embed-text";
      "digest.log" = "on";
      "history.keep_days" = "30";
      "scope.work.paths" = "~/Developer/acme";
      "scope.work.embedder" = "local";
    };
    claude = "/Users/me/.local/bin/claude"; # null: look on the activation PATH
    codex = null;
    index = {
      enable = true; # the timer needs embedder.url
      every = 15;
    };
    watch.enable = true; # default; false passes --no-watch
  };
}
```

`settings` takes the keys in [Configuration](reference/configuration.md#keys),
the `sync.*` ones included. Scope keys are quoted attribute names, as above.
`package` defaults to this flake's `bilbo` for the system.

Combining `index.enable` with `embedder.token_env` fails evaluation, because the
timer does not inherit your shell's environment and cannot read a key from a
variable. See [Keep the key](guides/embedders.md#keep-the-key). Use
`embedder.token_file`.

To run the [local embedder](guides/embedders.md#run-the-local-embedder) instead,
leave `embedder.url` and `embedder.model` unset and enable it:

```nix
programs.bilbo.localEmbedder.enable = true;
```

It writes the local URL, the model and the Qwen query prefix into the config
itself. It also takes `port` (default 8737) and `llamaServer` (default nixpkgs'
`llama-server`). Setting another `embedder.url` or `embedder.model` alongside it
fails evaluation.

When a module writes the config, `bilbo setup` reports it as managed elsewhere
and leaves it alone.

## Remove bilbo

```sh
bilbo setup --remove
```

This unloads the timer, the watcher and the local embedder's service, removes
the plugin and its marketplace from Claude Code and Codex, and takes away
Codex's trust of the hooks. It keeps the store and its history, the config, the
key file and the downloaded model, and prints their paths. In a terminal it asks
first; `--yes` skips the question.

To delete bilbo itself, delete the binary, the receipt and `env.*` files beside
it (listed under [What the installer writes](#what-the-installer-writes)), the
lines the installer added to your shell rc files, and, if you want, the paths
that `setup --remove` printed.

## See also

- [Getting started](getting-started.md)
- [Set up](guides/setup.md)
