# Configuration

The config file, its keys, the folders bilbo uses and the environment variables it reads. `bilbo config` lists every key with its value and where the value came from; see [Commands](commands.md#config).

## The config file

The config file is `$BILBO_CONFIG`, else `$XDG_CONFIG_HOME/bilbo/config.toml`, else `~/.config/bilbo/config.toml`. It is TOML; bilbo only reads it, and you edit it by hand. Each key sits in the table named before its dot:

```toml
[paths]
root = "~/bilbo/store"

[store]
seal_at = "16 MiB"
compression = 12
```

A key takes the first of these that sets it: a flag, its variable, the config file, its default. A variable set to the empty string counts as unset. No key has a flag yet. The config file is checked whole whatever the variables say: a bad value in it is a config error even for a key whose variable is set. A value bilbo refuses is a config error wherever it is set, even when a flag or a variable above it sets the key too.

A missing config file means every key takes its variable or its default, unless `BILBO_CONFIG` names the file: then the missing file is a config error.

Sizes are strings with a binary unit, `KiB`, `MiB` or `GiB`, with or without the space (`"8 MiB"`, `"512KiB"`), or an integer of bytes (`8388608`). Decimal units such as `MB` or `M` are refused. Paths are absolute, or start with `~/`, which stands for `$HOME`; a path with a `.` or `..` component is refused.

## Keys

| Key | Variable | Default | Meaning |
| --- | --- | --- | --- |
| `paths.root` | `BILBO_HOME` | `$XDG_DATA_HOME/bilbo, else ~/.local/share/bilbo` | The store: its sealed segments. Must be an absolute path or one starting with ~/. A root that holds a `notes/` or `library/` folder, bilbo 0.20's layout, is refused. |
| `paths.state` | none | `$XDG_STATE_HOME/bilbo, else ~/.local/state/bilbo` | This device's state: `open.seg`, the store lock and its store root. Must be an absolute path or one starting with ~/. |
| `paths.cache` | none | `$XDG_CACHE_HOME/bilbo, else ~/.cache/bilbo` | What bilbo rebuilds from the store: the catalog. Must be an absolute path or one starting with ~/. |
| `store.seal_at` | `BILBO_STORE_SEAL_AT` | `"8 MiB"` | The size at which `open.seg` is sealed into a segment of the store. Must be between 1 MiB and 1 GiB. |
| `store.compression` | `BILBO_STORE_COMPRESSION` | `19` | The zstd level of every frame the store writes: higher packs smaller and writes slower. Must be a zstd level from 1 to 22. |
| `store.tear_budget` | `BILBO_STORE_TEAR_BUDGET` | `4` | How many times `open.seg`'s size the search after a torn frame may hash before it refuses the file. Must be a whole number from 1 to 64. |

An XDG variable moves a default only when it holds an absolute path.

## Folders

| Folder | Path | Holds |
| --- | --- | --- |
| Store root | `paths.root` | The sealed segments, each `<ULID>.seg`. |
| State | `paths.state` | `open.seg`, `store.lock`, `sealing` and `root`. |
| Cache | `paths.cache` | `catalog.sqlite`. Deleting it loses nothing: the next run rebuilds it. |
| Config | `$BILBO_CONFIG`, else `$XDG_CONFIG_HOME/bilbo/config.toml`, else `~/.config/bilbo/config.toml` | The config file. |

The state and cache folders cannot be inside the store root.

Until the migration, point `BILBO_HOME` or `paths.root` at another folder. The default folders are bilbo 0.20's: it keeps its notes in `notes/` and `library/` under the root and reads its config from `config` in the same folder. This version reads only `config.toml`, and refuses a root that holds either folder before it writes anything, naming the folder: `<root>/notes belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder`.

## Errors

A key the [Keys](#keys) table does not list, a key set twice, a value of the wrong type, a value out of its range, a file that is not UTF-8, a file that is not TOML, a root that holds bilbo 0.20's folders and a state or cache folder inside the root are config errors. bilbo prints one line and exits 2. What the line names depends on the error:

- A value or a key in the file: the file, the line and the key.
- A file that is not TOML, or not UTF-8: the file and the line.
- A variable: the variable.
- A `BILBO_CONFIG` that names a missing file: the path, with no line or key.
- A `~/` value when `HOME` is unusable: the value's file, line and key, or its variable, then `needs HOME, which` and the fault: not set, empty, not an absolute path, or holding a `.` or `..` component.
- A root that holds bilbo 0.20's `notes/` or `library/`: that folder, and the settings to change.
- A state or cache folder inside the root, or the root itself, whether the path says so or a link leads there: `the state folder <path> is inside the store root <root>, which holds only segments; set paths.state to a folder outside the root`, or the same for the cache and `paths.cache`.
- A config path that is not a regular file, such as a folder or a named pipe: the path, as `cannot read <path>: not a regular file`.

```text
bilbo: /home/me/.config/bilbo/config.toml:7: store.seal_at must be between 1 MiB and 1 GiB, got "0"
bilbo: /home/me/.config/bilbo/config.toml:9: unknown key store.sealat; the keys are paths.root, paths.state, paths.cache, store.seal_at, store.compression, store.tear_budget
bilbo: /home/me/.config/bilbo/config.toml:4: duplicate key seal_at
bilbo: BILBO_STORE_SEAL_AT must be between 1 MiB and 1 GiB, got "0"
bilbo: /home/me/.config/bilbo/config.toml:2: not valid UTF-8
bilbo: BILBO_CONFIG names a file that does not exist: /home/me/bilbo.toml
bilbo: BILBO_HOME needs HOME, which is not set
bilbo: /home/me/.local/share/bilbo/notes belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder
```

`bilbo config` reports the same errors, except the 0.20-root refusal and the nesting refusal, which a verb runs when it opens the store. `bilbo config` runs neither. So it is the way to check a file.

## Environment variables

### Settings

| Variable | Effect |
| --- | --- |
| `BILBO_HOME` | Sets `paths.root`. |
| `BILBO_STORE_SEAL_AT`, `BILBO_STORE_COMPRESSION`, `BILBO_STORE_TEAR_BUDGET` | Set `store.seal_at`, `store.compression` and `store.tear_budget`. |
| `BILBO_CONFIG` | The config file's path. A file it names must exist. |
| `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` | Move the default store root, config file, cache and state folders, as [Keys](#keys) and [Folders](#folders) show. |
| `HOME` | The base of every default folder, and what `~/` stands for. |

### Terminal view

| Variable | Effect |
| --- | --- |
| `AI_AGENT`, `CLAUDE_CODE_CHILD_SESSION`, `CODEX_THREAD_ID`, `CODEX_CI`, `ANTIGRAVITY_AGENT` | When one is set and not empty, an agent runs bilbo: it prints the plain view on a terminal too. `CLAUDECODE` is not one. |
| `NO_COLOR`, `CLICOLOR` | No escapes in the terminal view when `NO_COLOR` is set and not empty, or `CLICOLOR` is `0`. |
| `TERM` | Unset, empty or `dumb`: no escapes in the terminal view. |
| `LC_ALL`, `LC_CTYPE`, `LANG` | Outside macOS, the terminal view's marks are ASCII unless the first of them that is set and not empty holds a codeset of `UTF-8` or `utf8`, in any case, as in `en_US.UTF-8` or `de_DE.utf8@euro`. On macOS the marks are always Unicode. |

## See also

- [Commands](commands.md)
