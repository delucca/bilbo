# Configuration

The config file, its keys, the folders bilbo uses and the environment variables it reads. `bilbo config` lists every key with its value and where the value came from; see [Commands](commands.md#config).

## The config file

The config file is `$BILBO_CONFIG`, else `$XDG_CONFIG_HOME/bilbo/config.toml`, else `~/.config/bilbo/config.toml`. It is TOML, and you edit it by hand; the one thing bilbo writes to it is a `[[follow]]` table that `bilbo remember --follow` appends (see [Commands](commands.md#remember)), taking turns with other saves on `config.toml.lock` beside it, which it leaves there. Each key sits in the table named before its dot, and each follow is a `[[follow]]` table of its own:

```toml
[paths]
root = "~/bilbo/store"

[store]
seal_at = "16 MiB"
compression = 12

[[follow]]
path = "~/.claude/projects"
kind = "session"

[[follow]]
path = "~/notes"
```

A key takes the first of these that sets it: a flag, its variable, the config file, its default. A list such as `follow` is taken whole from that one source: a file that declares one follow drops the default follows. A variable set to the empty string counts as unset. One key has a flag: `--budget` of `bilbo read` sets `read.budget`. The config file is checked whole whatever the variables say: a bad value in it is a config error even for a key whose variable is set. A value bilbo refuses is a config error wherever it is set, even when a flag or a variable above it sets the key too.

A missing config file means every key takes its variable or its default, unless `BILBO_CONFIG` names the file: then the missing file is a config error.

Sizes are strings with a binary unit, `KiB`, `MiB` or `GiB`, with or without the space (`"8 MiB"`, `"512KiB"`), or an integer of bytes (`8388608`). Decimal units such as `MB` or `M` are refused. Durations are strings of whole seconds, minutes or hours, with or without the space (`"90s"`, `"60m"`, `"1 h"`); a bare number is refused. Paths are absolute, or start with `~/`, which stands for `$HOME`; a path with a `.` or `..` component is refused.

## Keys

| Key | Variable | Default | Meaning |
| --- | --- | --- | --- |
| `paths.root` | `BILBO_HOME` | `$XDG_DATA_HOME/bilbo, else ~/.local/share/bilbo` | The store: its sealed segments. Must be an absolute path or one starting with ~/. A root that holds a `notes/` or `library/` folder, bilbo 0.20's layout, is refused. |
| `paths.state` | none | `$XDG_STATE_HOME/bilbo, else ~/.local/state/bilbo` | This device's state: `open.seg`, the store lock and its store root. Must be an absolute path or one starting with ~/. |
| `paths.cache` | none | `$XDG_CACHE_HOME/bilbo, else ~/.cache/bilbo` | What bilbo rebuilds from the store: the catalog. Must be an absolute path or one starting with ~/. |
| `store.seal_at` | `BILBO_STORE_SEAL_AT` | `"8 MiB"` | The size at which `open.seg` is sealed into a segment of the store. Must be between 1 MiB and 1 GiB. A run checks it after each capture and when it ends. |
| `store.seal_after` | `BILBO_STORE_SEAL_AFTER` | `"60m"` | The age of `open.seg`'s oldest frame at which a run seals it, whatever its size. Must be between 1m and 24h. A run checks it when it ends, and a bare run does so even when no follow claims a file; `bilbo remember --hook` never seals. |
| `store.compression` | `BILBO_STORE_COMPRESSION` | `19` | The zstd level of every frame the store writes: higher packs smaller and writes slower. Must be a zstd level from 1 to 22. |
| `store.tear_budget` | `BILBO_STORE_TEAR_BUDGET` | `4` | How many times `open.seg`'s size the search after a torn frame may hash before it refuses the file. Must be a whole number from 1 to 64. |
| `store.wait` | `BILBO_STORE_WAIT` | `"10s"` | How long a reader of the store, such as bilbo read, waits for a bilbo that is writing it before it gives up. Must be between 0s and 10m. A reader that gives up exits 1 and says so; `"0s"` never waits. |
| `store.hook_wait` | `BILBO_STORE_HOOK_WAIT` | `"2s"` | How long bilbo remember --hook waits for a bilbo that is reading the store before it leaves the lines to the next run. Must be between 0s and 4s. A harness gives a hook five seconds; a hook that gives up says so in one line on stderr and exits 0. |
| `follow` | none | `[{ path = "~/.claude/projects", kind = "session" }, { path = "~/.codex/sessions", kind = "session" }]` | What a bare bilbo remember keeps up with: each folder or file, and the kind that runs there, else the one detected. Must be a list of [[follow]] tables, each with a path and an optional kind. `--follow` refuses a file whose `follow` is an inline array and prints the inline table to add. A path follows the rules of the folder keys. A follow whose folder is missing is skipped with one line on stderr. |
| `read.budget` | `BILBO_READ_BUDGET` | `7000` | The most tokens a page of bilbo read prints, as bilbo counts them. The default keeps a page written to a pipe or a file under the 30,000 characters at which Claude Code cuts a command's output. Must be a whole number of 256 or more. The flag `--budget` of `bilbo read` sets it for one run. |

An XDG variable moves a default only when it holds an absolute path.

## Folders

| Folder | Path | Holds |
| --- | --- | --- |
| Store root | `paths.root` | The sealed segments, each `<ULID>.seg`. |
| State | `paths.state` | `open.seg`, `open.since` (when `open.seg` got its oldest frame), `store.lock`, `store.writer` and `store.again` (the store lock, the writer's lock and the again flag; see [Commands](commands.md#remember)), `sealing` and `root`. |
| Cache | `paths.cache` | `catalog.sqlite`. Deleting it loses nothing: the next `bilbo remember` with a target or none rebuilds it; a hook leaves that to such a run, and `bilbo read` reads the segments themselves until then, which is slower. bilbo trusts the catalog for the store it was built from: after restoring a store from a copy, delete it. |
| Config | `$BILBO_CONFIG`, else `$XDG_CONFIG_HOME/bilbo/config.toml`, else `~/.config/bilbo/config.toml` | The config file. |

The state and cache folders cannot be inside the store root.

Until the migration, point `paths.root` (or `BILBO_HOME`), `paths.state` and `paths.cache` at folders of their own. The default folders are bilbo 0.20's: it keeps its notes in `notes/` and `library/` under the root and reads its config from `config` in the same folder. This version reads only `config.toml`, and refuses a root that holds either folder before it writes anything, naming the folder: `<root>/notes belongs to bilbo 0.20; set BILBO_HOME or paths.root to another folder`.

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
bilbo: /home/me/.config/bilbo/config.toml:9: unknown key store.sealat; the keys are paths.root, paths.state, paths.cache, store.seal_at, store.seal_after, store.compression, store.tear_budget, store.wait, store.hook_wait, follow, read.budget
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
| `BILBO_STORE_SEAL_AT`, `BILBO_STORE_SEAL_AFTER`, `BILBO_STORE_COMPRESSION`, `BILBO_STORE_TEAR_BUDGET`, `BILBO_STORE_WAIT`, `BILBO_STORE_HOOK_WAIT` | Set `store.seal_at`, `store.seal_after`, `store.compression`, `store.tear_budget`, `store.wait` and `store.hook_wait`. |
| `BILBO_READ_BUDGET` | Sets `read.budget`. |
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
