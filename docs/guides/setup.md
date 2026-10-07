# Set up bilbo

This guide shows what `bilbo setup` does and how to run it: in a terminal with
the wizard, or from a script or an agent with flags. It also covers the watch
service and the sync step.

`bilbo setup` plans every step, shows the plan, asks once, then applies it and
prints one line per step (`created`, `written`, `kept`, `installed`, `failed`,
and so on). It does these things:

- creates the store, `<root>/notes/`;
- writes the config, after checking the embedder with one real request;
- installs the bilbo plugin in Claude Code and Codex, at the binary's own
  version, for each of the two that is on your `PATH`;
- trusts the plugin's hooks in Codex, which runs a plugin hook only once it is
  trusted: setup asks `codex app-server` to record the trust, so no review step
  is left, and a release that changes the hook is trusted again on the next run;
- installs a timer that runs `bilbo index` every 15 minutes (a launchd agent on
  macOS, a systemd user timer on Linux), when an embedder is configured, and
  appends its output to `index.log` under bilbo's state folder, each line
  starting with the time;
- installs a login service that runs `bilbo watch`, which records [note
  history](history.md) and [syncs](sync.md) the scopes that sync;
- checks each syncing scope's folder, and in the wizard can turn sync on.

With the local embedder ([Embedders](embedders.md#run-the-local-embedder)),
setup also downloads a model and installs a login service that runs it. Their
steps, `model` and `server`, are always in the report, as `skipped` without it.

Run it again at any time. With the same inputs every step that did something is
`kept`. It exits 1 when any step failed, and a failed step does not stop the
ones after it. The report is one line per step, as the tutorial shows; in a
terminal it is a marked step list with a summary line.

## Run the wizard in a terminal

With stdin and stderr on a terminal and no flags, `bilbo setup` runs a wizard.
It offers no embedder (keyword search only), a local embedder run by bilbo,
Ollama found on `localhost:11434`, OpenAI, or another OpenAI-compatible URL. For
a key, it takes the name of an environment variable, a key file, or a pasted key
with hidden input, saved to `<config folder>/token` with mode 0600. On a rerun
it shows the current values as defaults, and afterwards it offers to run the
first `bilbo index`. `--interactive` forces the wizard.

## Run setup from a script or an agent

Without a terminal, with `--yes`, or when a flag answers a question, setup asks
nothing and a missing answer takes its default:

```sh
bilbo setup --yes \
  --embedder-url http://localhost:11434 \
  --embedder-model nomic-embed-text
```

To try setup without installing anything, add `--no-plugin --no-timer
--no-watch`.

### Options

| Option | Meaning |
| --- | --- |
| `--embedder-url <url>`, `--embedder-model <name>` | The embedder. They go together. |
| `--embedder-local` | Run the [local embedder](embedders.md#run-the-local-embedder). |
| `--embedder-port <n>` | The local embedder's port on `127.0.0.1`, default 8737. |
| `--llama-server <path>` | The `llama-server` to run, instead of the one on `PATH`. |
| `--embedder-token-env <var>`, `--embedder-token-file <path>` | Where the key lives. Pick one. No option takes the key itself. |
| `--embedder-query-prefix <text>` | Text put before each query. |
| `--no-plugin` | Skip the agent plugin. |
| `--claude <path>`, `--codex <path>` | Use this executable instead of looking one up on `PATH`. |
| `--plugin-source <folder\|owner/repo#ref>` | Install the plugin from here instead of the default source. |
| `--no-timer` | Skip the index timer. |
| `--index-every <minutes>` | Timer interval, 1 to 1440. |
| `--no-watch` | Skip the history watcher, and remove it when installed. |

`--yes`, `--interactive` and `--remove` are covered above and in
[Install](../install.md#remove-bilbo). `--interactive` with `--yes` or with an
answer flag is a usage error, and so is `--interactive` without a terminal.

The index timer cannot read a key from a variable; see [Keep the
key](embedders.md#keep-the-key).

## Watch service

Unless `--no-watch` is given or the wizard's answer declines it, setup installs
a login service that runs `bilbo watch`, restarts it when it exits and appends
its output to `watch.log` under bilbo's state folder: the launchd agent
`io.github.delucca.bilbo.watch` on macOS, the systemd user service
`bilbo-watch.service` on Linux. It needs no embedder and no network, and a key
in an environment variable does not fail it. Each line of `watch.log` starts
with the time, as `2026-10-07T01:02:03-03:00 bilbo: watching ...`.

The step is the `watch` line of the report, after `timer`. In the wizard it is
one question, "Record note history in the background?", defaulting to yes.

## Turn on sync in setup

The `sync` line of the report, after `watch`, checks each scope whose `sync` is
a URL: that this device has a key, that the watcher is wanted and that the
transport answers (a folder exists and is writable, a [relay](relay.md) URL
answers as a bilbo relay). It reports:

- `ok: personal through file:///Users/me/Dropbox/bilbo (42 notes)`
- `failed: sync needs the watcher; drop --no-watch`
- `failed: <url> is not reachable: <reason>`
- `failed: <url> is not a bilbo relay`
- `skipped: no device key; run bilbo device init in a terminal`
- `skipped: no scope syncs`

It writes nothing.

In the wizard, after the watcher's question, one more asks whether to sync notes
between devices. On yes it asks for the scope and where to sync: a folder
(absolute or starting with `~/`, its parent existing) or a relay URL. Then, when
the device has no key, it asks whether you already have a recovery phrase: yes
runs `bilbo device recover`, no runs `bilbo device init`, with the same terminal
rules as [the recovery phrase](devices.md#commands-that-need-a-terminal).
Nothing is written until you confirm the summary. It then writes
`scope.<name>.sync`, creates the folder's last component when missing and writes
the keys. Against a config that a module manages, the wizard only shows the
syncing scopes.

## See also

- [Install](../install.md) for the home-manager module and for removing bilbo.
- [Embedders](embedders.md)
- [Troubleshooting](../troubleshooting.md) for `failed` lines.
