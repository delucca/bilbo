# bilbo

Durable memory for coding agents: Markdown notes an agent writes in one
session and finds again in the next.

Agents re-derive what an earlier session already worked out. bilbo gives them
a store of plain Markdown notes they write with their own file tools, and a
`bilbo` command that starts, checks, searches and indexes those notes. A
`recall` skill for Claude Code and Codex puts the search in the agent's hands.

- **Plain files.** One note per topic in `<root>/notes/`, named
  `<kind>-<topic>.md`, with a small YAML frontmatter. Read, edit, grep or
  version them like any other file.
- **Keyword search out of the box, meaning search with an embedder.** Point
  bilbo at Ollama, OpenAI or any OpenAI-compatible URL, or let it run a local
  model, and `recall` finds notes that share no word with the query.
- **Agent plugin.** `bilbo setup` installs the plugin in Claude Code and Codex
  and a timer that keeps the index current.

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Usage](#usage)
- [Set up](#set-up)
- [Configuration](#configuration)
- [Contributing](#contributing)
- [License](#license)

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

## Quick start

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/delucca/bilbo/releases/latest/download/bilbo-installer.sh | sh && bilbo setup
```

In a terminal, `bilbo setup` asks which embedder to use, then creates the
store and installs the plugin. Ask your agent to "recall" something, or try the
command yourself:

```sh
bilbo new gotcha sqlite-busy-timeout --title "SQLite needs a busy timeout"
# edit the file it prints, then:
bilbo recall busy timeout
```

## Usage

### Notes

A note is one Markdown file in `<root>/notes/`, named `<kind>-<topic>.md`. The
kind is one of `plan`, `spec`, `design`, `decision`, `gotcha`, `research`,
`review`, `report` or `reference`; the topic is lowercase kebab-case, and a
topic has at most one note whatever its kind. `bilbo new` writes the
frontmatter and the title; the agent writes the rest:

```markdown
---
id: 01M419P9SCTSHK92P636TZR72R
created: 2026-10-03T13:31-03:00
sources:
  - "url: https://www.sqlite.org/c3ref/busy_timeout.html"
---

# SQLite needs a busy timeout

Two writers on the same database file fail with `SQLITE_BUSY` unless each
connection sets `PRAGMA busy_timeout`.

## Fix

Set `busy_timeout = 5000` right after opening the connection.
```

`id` is a ULID and `created` the local time to the minute. `sources` is
optional, each item a quoted `<type>: <value>` with the type `url`, `code`,
`doc` or `search`. No other key is allowed. The store root is `$BILBO_HOME`,
else `$XDG_DATA_HOME/bilbo`, else `~/.local/share/bilbo`, on macOS too.

### Commands

| Command | What it does |
| --- | --- |
| `bilbo new <kind> <topic> [--title <text>]` | Creates the note and prints its path. Exits 1 when the topic already has a note. |
| `bilbo check` | Prints every problem in the store, one per line, and changes nothing. Exits 1 when it finds any. |
| `bilbo recall <query>... [--kind <kind>]... [--limit <n>]` | Prints the notes that best match, best first, 10 by default. Exits 1 when nothing matches. |
| `bilbo index` | Embeds the passages the vector cache lacks and drops the ones no note holds any more. |
| `bilbo setup` | See [Set up](#set-up). |

`recall` prints one block per note: the path and line of the best passage, the
kind and `created` (tab-separated), then the passage's heading path, then the
first 300 characters of its text:

```console
$ bilbo recall busy timeout
/Users/me/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:13	gotcha	2026-10-03T13:31-03:00
SQLite needs a busy timeout > Fix
Set `busy_timeout = 5000` right after opening the connection.
```

Without an embedder, `recall` matches whole words, ignoring case and accents.
With one, it fuses that order with a ranking by meaning. When the embedder is
down it falls back to keywords and says so on stderr; passages written since
the last `bilbo index` rank by keywords only, and `recall` says that too.

`check` lints the whole store against the note rules, so mistakes an agent
makes while editing files by hand surface without bilbo blocking anything:

```console
$ bilbo check
notes/plan-broken.md: created: missing
notes/plan-broken.md: title: missing; add one '# <title>' line after the frontmatter
```

`bilbo --help` prints the full usage.

### From an agent

The bilbo plugin gives Claude Code and Codex a `recall` skill. It runs
`bilbo recall` with the user's words, retries twice in the note's likely
wording when nothing matches, and offers to open a hit. It searches through
`bilbo` only: when the binary is missing, it says so and stops.

## Set up

`bilbo setup` plans every step, shows the plan, asks once, then applies it and
prints one line per step (`created`, `written`, `kept`, `installed`, `failed`,
and so on). It does four things:

- creates the store, `<root>/notes/`;
- writes the config, after checking the embedder with one real request;
- installs the bilbo plugin in Claude Code and Codex, at the binary's own
  version, for each of the two that is on your `PATH`;
- installs a timer that runs `bilbo index` every 15 minutes (a launchd agent on
  macOS, a systemd user timer on Linux), when an embedder is configured.

With the local embedder (below), setup also downloads a model and installs a
login service that runs it. Their steps, `model` and `server`, are always in
the report, as `skipped` without it.

Run it again at any time. With the same inputs every step is `kept`. It exits 1
when any step failed, and a failed step does not stop the ones after it.

### In a terminal

With stdin and stderr on a terminal and no flags, `bilbo setup` runs a wizard.
It offers no embedder (keyword search only), a local embedder run by bilbo,
Ollama found on `localhost:11434`, OpenAI, or another OpenAI-compatible URL.
For a key, it takes the name of an environment variable, a key file, or a
pasted key with hidden input, saved to `<config folder>/token` with mode 0600.
On a rerun it shows the current values as defaults, and afterwards it offers to
run the first `bilbo index`. `--interactive` forces the wizard.

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
| `--embedder-local` | Run the [local embedder](#local-embedder). |
| `--embedder-port <n>` | The local embedder's port on `127.0.0.1`, default 8737. |
| `--llama-server <path>` | The `llama-server` to run, instead of the one on `PATH`. |
| `--embedder-token-env <var>`, `--embedder-token-file <path>` | Where the key lives. Pick one. No option takes the key itself. |
| `--embedder-query-prefix <text>` | Text put before each query. |
| `--no-plugin` | Skip the agent plugin. |
| `--claude <path>`, `--codex <path>` | Use this executable instead of looking one up on `PATH`. |
| `--plugin-source <folder\|owner/repo#ref>` | Install the plugin from here instead of the default source. |
| `--no-timer` | Skip the index timer. |
| `--index-every <minutes>` | Timer interval, 1 to 1440. |

The timer does not inherit your shell's environment, so it cannot read a key
from a variable. Keep the key in a file (`--embedder-token-file`, or paste it
in the wizard); with `--embedder-token-env` the timer step fails and says so.

### Local embedder

Meaning ranking needs an embedder. If you have none, `bilbo setup --yes
--embedder-local` (or the wizard's second choice) runs one for you:

```sh
bilbo setup --yes --embedder-local [--embedder-port <n>] [--llama-server <path>]
```

Setup downloads a pinned model, Qwen3-Embedding-0.6B (639 MB, checked against
its SHA-256), to `models/` under bilbo's cache folder. It then installs a login
service, a launchd agent on macOS or a systemd user service on Linux, that runs
`llama-server` on `127.0.0.1` and restarts it if it exits. It waits for the
server, checks it with one real request, and points the config at it. An
interrupted download continues on the next run.

bilbo does not install `llama-server`. Have it on your `PATH`, or pass
`--llama-server`: `brew install llama.cpp` on macOS, your distribution's
`llama.cpp` package, or Nix. Linux needs a systemd user session.

The server stays loaded, about 1 GB of memory in use, so that `recall` never
waits for a model to load. The first `bilbo index` of a large store takes a
while. The server's log is `embedder.log` under bilbo's state folder.

Setup fails before it downloads anything when `llama-server` is missing or the
port is taken. If the config later moves to another embedder, a rerun removes
the service.

### Remove

```sh
bilbo setup --remove
```

This unloads the timer and the local embedder's service, and removes the plugin
and its marketplace from Claude Code and Codex. It keeps the store, the config,
the key file and the downloaded model, and prints their paths. In a terminal
it asks first; `--yes` skips the question.

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

`settings` takes the keys in [Configuration](#configuration). Combining
`index.enable` with `embedder.token_env` fails evaluation, for the reason
above: use `embedder.token_file`. `package` defaults to this flake's `bilbo`
for the system.

To run the [local embedder](#local-embedder) instead, leave `embedder.url` and
`embedder.model` unset and enable it:

```nix
programs.bilbo.localEmbedder.enable = true;
```

It writes the local URL, the model and the Qwen query prefix into the config
itself. It also takes `port` (default 8737) and `llamaServer` (default nixpkgs'
`llama-server`). Setting another `embedder.url` or `embedder.model` alongside it fails evaluation.

## Configuration

`bilbo setup` writes the config, and you can edit it by hand. It lives at
`$BILBO_CONFIG`, else `$XDG_CONFIG_HOME/bilbo/config`, else
`~/.config/bilbo/config`. It holds one `<key> = <value>` per line; blank lines
and lines starting with `#` are ignored:

```
embedder.url = http://localhost:11434
embedder.model = nomic-embed-text
```

| Key | Meaning |
| --- | --- |
| `embedder.url` | An `http` or `https` URL serving `/v1/embeddings`. Without it, bilbo is keyword-only. |
| `embedder.model` | The model name. Required with a URL. |
| `embedder.token_file`, `embedder.token_env` | Where the bearer token lives: a file (absolute or `~/`) or a variable. At most one. |
| `embedder.query_prefix` | Text put before every query. Empty by default. |
| `embedder.min_similarity` | How close a passage must be to enter the meaning ranking, 0 to 1. Default 0.5. |

bilbo never prints the token. The vector cache lives under
`$XDG_CACHE_HOME/bilbo`, else `~/.cache/bilbo`; deleting it loses nothing that
`bilbo index` cannot rebuild.

## Contributing

Pull requests are welcome. Behavior changes start as an
[OpenSpec](https://github.com/Fission-AI/OpenSpec) change under
[`openspec/changes/`](openspec/changes/), and [`openspec/specs/`](openspec/specs/)
holds the current contract for each command. [`AGENTS.md`](AGENTS.md) has the
commands CI runs and the rules the code follows. In short, with Nix:

```sh
nix develop -c cargo test --locked
```

## License

[Apache-2.0](LICENSE)
