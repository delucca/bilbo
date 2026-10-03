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

Or add it as a flake input and use `bilbo.packages.${system}.default`. With
home-manager, see [home-manager](#home-manager):

```nix
inputs.bilbo = {
  url = "github:delucca/bilbo";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

`inputs.nixpkgs-unstable.follows` is optional: only the dev shell reads that
input.

## Set up

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/delucca/bilbo/releases/latest/download/bilbo-installer.sh | sh && bilbo setup
```

`bilbo setup` plans every step, shows the plan, asks once, then applies it and
prints one line per step (`created`, `written`, `kept`, `installed`, `failed`,
and so on). It does four things:

- creates the store, `<root>/notes/`;
- writes the config, after checking the embedder with one real request;
- installs the bilbo plugin in Claude Code and Codex, at the binary's own
  version, for each of the two that is on your `PATH`;
- installs a timer that runs `bilbo index` every 15 minutes (a launchd agent on
  macOS, a systemd user timer on Linux), when an embedder is configured.

Run it again at any time. With the same inputs every step is `kept`. It exits 1
when any step failed, and a failed step does not stop the ones after it.

### In a terminal

With stdin and stderr on a terminal and no flags, `bilbo setup` runs a wizard.
It offers no embedder (keyword search only), Ollama found on
`localhost:11434`, OpenAI, or another OpenAI-compatible URL. For a key, it
takes the name of an environment variable, a key file, or a pasted key with
hidden input, saved to `<config folder>/token` with mode 0600. On a rerun it
shows the current values as defaults, and afterwards it offers to run the first
`bilbo index`. `--interactive` forces the wizard.

### In a script or from an agent

Without a terminal, with `--yes`, or when a flag answers a question, setup
asks nothing and a missing answer takes its default:

```sh
bilbo setup --yes \
  --embedder-url http://localhost:11434 \
  --embedder-model nomic-embed-text
```

| Option | Meaning |
| --- | --- |
| `--embedder-url <url>`, `--embedder-model <name>` | The embedder. They go together. |
| `--embedder-token-env <var>`, `--embedder-token-file <path>` | Where the key lives. Pick one. No option takes the key itself. |
| `--embedder-query-prefix <text>` | Text put before each query. |
| `--no-plugin` | Skip the agent plugin. |
| `--claude <path>`, `--codex <path>` | Use this executable instead of looking one up on `PATH`. |
| `--plugin-source <folder\|owner/repo#ref>` | Install the plugin from here instead of the default source. |
| `--no-timer` | Skip the index timer. |
| `--index-every <minutes>` | Timer interval, 1 to 1440. |

`bilbo --help` lists them all.

The timer does not inherit your shell's environment, so it cannot read a key
from a variable. Keep the key in a file (`--embedder-token-file`, or paste it
in the wizard); with `--embedder-token-env` the timer step fails and says so.

### Remove

```sh
bilbo setup --remove
```

This unloads the timer and removes the plugin and its marketplace from Claude
Code and Codex. It keeps the store, the config and the key file, and prints
their paths. In a terminal it asks first; `--yes` skips the question.

### home-manager

The flake has a home-manager module. It writes the config from `settings` and
runs `bilbo setup --yes` on activation:

```nix
inputs.bilbo = {
  url = "github:delucca/bilbo";
  inputs.nixpkgs.follows = "nixpkgs";
  inputs.home-manager.follows = "home-manager";
};
```

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
    };
    claude = "/Users/me/.local/bin/claude"; # null: look on the activation PATH
    codex = null;
    index = {
      enable = true; # the timer needs embedder.url
      every = 15;
    };
  };
}
```

`settings` takes `embedder.url`, `embedder.model`, `embedder.token_file`,
`embedder.token_env`, `embedder.query_prefix` and `embedder.min_similarity`.
Combining `index.enable` with `embedder.token_env` fails evaluation, for the
reason above: use `embedder.token_file`. `package` defaults to this flake's
`bilbo` for the system.
