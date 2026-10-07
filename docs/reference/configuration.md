# Configuration

The config file, its keys, the folders bilbo uses and the environment variables
it reads. `bilbo setup` writes the config for you, and you can edit it by hand.

## The config file

The config lives where [Folders](#folders) says. It holds one `<key> = <value>`
per line; blank lines and lines starting with `#` are ignored:

```
embedder.url = http://localhost:11434
embedder.model = nomic-embed-text
```

bilbo never prints the token.

## Keys

| Key | Meaning |
| --- | --- |
| `embedder.url` | An `http` or `https` URL serving `/v1/embeddings`. Without it, bilbo is keyword-only. |
| `embedder.model` | The model name. Required with a URL. |
| `embedder.token_file`, `embedder.token_env` | Where the bearer token lives: a file (absolute or `~/`) or a variable. At most one. |
| `embedder.query_prefix` | Text put before every query. Empty by default. |
| `embedder.min_similarity` | How close a passage must be to enter the meaning ranking, 0 to 1. Default 0.5. |
| `digest.enable` | `off` turns [the digest](../guides/agents.md#the-digest) off: the hook prints nothing and writes nothing. `on` by default. |
| `digest.min_similarity` | How close a passage must be to enter the digest when an embedder answers, 0 to 1. Default 0.55. |
| `digest.log` | `on` appends each digest run to the digest log. `off` by default. |
| `scope.<name>.sync`, `scope.<name>.embedder`, `scope.<name>.paths`, `scope.<name>.marks` | Declare the scope `<name>`; see [Scopes](../guides/scopes.md#declare-a-scope). `sync` is `off` or a [URL](../guides/sync.md#sync-urls), `embedder` is `any` or `local`, the others comma-separated lists. |
| `scope.default` | The declared scope `bilbo new` falls back on. |
| `sync.poll_seconds` | A whole number of seconds, 1 to 3600: how often `bilbo watch` looks for other devices' files; see [Sync](../guides/sync.md). 30 by default. |
| `sync.stale_days` | A whole number of days, 1 to 3650: how long a device may leave a segment unacknowledged before `bilbo sync` calls it stale and pruning stops waiting for it. 180 by default. |
| `history.keep_days` | A whole number of days, 1 to 3650: the age past which `bilbo watch` prunes versions, under [the retention rule](../guides/history.md#retention). 90 by default. |

The embedder rule that `scope.<name>.embedder` sets is in [Search by meaning
with an embedder](../guides/embedders.md#the-embedder-rule).

## Folders

| Folder | Path | Holds |
| --- | --- | --- |
| Store root | `$BILBO_HOME`, else `$XDG_DATA_HOME/bilbo`, else `~/.local/share/bilbo`, on macOS too | The notes, the library and `.bilbo/`; see [Files and formats](files.md#store-layout). |
| Config | `$BILBO_CONFIG`, else `$XDG_CONFIG_HOME/bilbo/config`, else `~/.config/bilbo/config` | The config file. |
| Cache | `$XDG_CACHE_HOME/bilbo`, else `~/.cache/bilbo` | The vector cache, the digest's `sessions/` and the local embedder's `models/`. |
| State | `$XDG_STATE_HOME/bilbo`, else `~/.local/state/bilbo` | The device keys in `keys/`, library stages and `plans/`, `digest.jsonl`, `watch.log` and `embedder.log`. |

Deleting the cache loses nothing that `bilbo index` cannot rebuild. `bilbo
--help` prints the rule for each of these four paths.

## Environment variables

| Variable | Effect |
| --- | --- |
| `BILBO_HOME` | The store root. |
| `BILBO_CONFIG` | The config file's path. |
| `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` | Move the store root, the config folder, the cache folder and the state folder, as the table above shows. |
| `XDG_BIN_HOME` | Where the installer puts the binary, else `~/.local/bin`; see [Install](../install.md). |
| `BILBO_NO_MODIFY_PATH` | Set to `1` to stop the installer editing your shell rc files; see [Install](../install.md). |
| `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and their lowercase forms | bilbo ignores them for a loopback embedder, which it reaches directly. |
| `AI_AGENT`, `CLAUDE_CODE_CHILD_SESSION`, `CODEX_THREAD_ID`, `CODEX_CI` | When one is set and not empty, an agent runs bilbo: it prints the plain view on a terminal too, and refuses the steps that need the user at a terminal; see [Commands that need a terminal](../guides/devices.md#commands-that-need-a-terminal). `CLAUDECODE` is not one. |
| `NO_COLOR`, `CLICOLOR` | No escapes in the terminal view when `NO_COLOR` is set and not empty, or `CLICOLOR` is `0`; `CLICOLOR_FORCE` and `FORCE_COLOR` are ignored. See [Terminal output](commands.md#terminal-output). |
| `TERM` | Unset or `dumb`: no escapes in the terminal view. |
| `COLUMNS` | The width of the terminal view when the terminal does not report one. |
| `LANG` | Outside macOS, the terminal view's marks are ASCII unless it ends in `UTF-8`. |
| The variable `embedder.token_env` names | The embedder's bearer token; the index timer cannot read it, see [Keep the key](../guides/embedders.md#keep-the-key). |

## See also

- [Commands](commands.md)
- [Set up bilbo](../guides/setup.md), which writes the config.
