# Commands

## Every verb

Results go to standard output; diagnostics go to standard error, each line prefixed `bilbo: ` off a terminal.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | refused, problems found or nothing matched |
| 2 | usage or config error |

## config

```text
bilbo config [<key>]
```

Shows every setting: its key, its value and where the value came from. With a key, it prints that key's value alone, for scripts. It never changes a setting: edit the config file. [Configuration](configuration.md) lists the keys, their variables and their defaults.

### Output

Standard output, one line per key, in the order [Configuration](configuration.md#keys) lists them:

```text
paths.root = "/home/me/.local/share/bilbo" (default)
paths.state = "/home/me/.local/state/bilbo" (default)
paths.cache = "/home/me/.cache/bilbo" (default)
store.seal_at = "16 MiB" (/home/me/.config/bilbo/config.toml:5)
store.seal_after = "60m" (default)
store.compression = 3 (BILBO_STORE_COMPRESSION)
store.tear_budget = 4 (default)
store.wait = "10s" (default)
store.hook_wait = "2s" (default)
follow = [{ path = "/home/me/.claude/projects", kind = "session" }, { path = "/home/me/.codex/sessions", kind = "session" }] (default)
read.budget = 7000 (default)
```

The value is TOML, as the config file would hold it. The origin is `default`, the config file and the line that set the key, the variable, or the flag. On a terminal the values line up and the origins are dimmed.

`bilbo config store.seal_at` prints `16 MiB`: a string without its quotes, any other value as TOML. A key bilbo does not know is a usage error.

A config file or variable that bilbo refuses fails `bilbo config` as it fails every other verb, with the same message, except the 0.20-root refusal and the refusal of a state or cache folder inside the store root, which a verb runs when it opens the store (`the state folder <path> is inside the store root <root>, which holds only segments; set paths.state to a folder outside the root`). `bilbo config` runs neither. So `bilbo config` checks a file you edited.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 2 | usage or config error: an unknown key, or a config file or variable bilbo refuses |

## remember

```text
bilbo remember <file|folder> [--kind <kind>] [--follow]
bilbo remember
bilbo remember --hook
```

Keeps files as entries in the store. An entry is a sequence of records, numbered from 1 per generation. Run it again and it keeps only what is new. With a target, it keeps that file or folder; with none, it keeps every follow; with `--hook`, it keeps the transcript a harness hook names.

### Kinds

A kind picks the files under the target, splits each into records and proposes the entry's meta. `bilbo remember --help` lists the kinds.

| Kind | Claims | Records |
|---|---|---|
| `markdown` | `.md` files | one per line, newline included; a final line without a newline is one record |
| `session` | files whose name ends in a UUID and `.jsonl` (Claude Code's `<uuid>.jsonl`, Codex's `rollout-<time>-<uuid>.jsonl`), and every `agent-*.jsonl` under Claude Code's `<uuid>/subagents/`, workflow runs' included | one per complete line, newline included; a line still being written waits for a later run |

A `markdown` file that is not UTF-8 is refused. A valid ULID in the frontmatter `id` becomes the entry's id at its first capture; a file with no usable `id` gets a fresh one. An id already held by an entry of another origin is refused for that file.

A `session` folder walk skips Claude Code's `memory/`, `workflows/` and `tool-results/` folders (a project's memories, a session's workflow definitions and its large tool outputs), but nothing below a `subagents/` folder, where `workflows/wf_*/` holds the subagents of workflow runs. It never claims the `.meta.json` beside a subagent, and never reads it: it names no session. Main sessions are kept before subagents, so a subagent's entry names its session's entry as its parent: the session `<uuid>.jsonl` in the folder that holds `<uuid>/subagents/`, however deep below it the subagent sits. A subagent kept before its session, as a target of its own, gets its parent the next time a run actually reads it. Growth is one cause of a read, not the only one. A run skips a file only while the catalog keeps a trustworthy stamp for it, and a stamp taken within two seconds of the file's last write is not kept. Which harness wrote a file, Claude Code, Codex or Cursor (whose Claude-shaped lines carry `role` and no `type`), is read from its lines each time and never stored. A file that is not UTF-8, or whose lines are none of these, is refused.

### Detection

Without `--kind`, every kind says which files it claims under the target. Exactly one kind claims files: it runs. None: there is nothing to keep (exit 1). Several: a usage error that names them and asks for `--kind`. With `--kind`, only that kind runs; an unknown kind is a usage error. `~/.claude/projects` holds `memory/*.md` files, so it needs `--kind session`.

### What the walk skips

A folder is walked recursively. Hidden entries (names starting with `.`) and symbolic links that the walk finds are skipped, and the walk never follows a link, so it cannot loop.

A target named on the command line is taken as given, even when it is a symbolic link or a hidden name, as `find -H` and `git add` treat their arguments. Its origin is the resolved path, and a kind tests the resolved file, not the name given: a `note.md` link to a file without `.md` keeps nothing.

### Outcomes

An entry is found again by its origin (`file:<absolute path>`). For each file:

- **new**: no entry has this origin yet.
- **appended**: the earlier bytes are unchanged and the file gained records; only those are kept.
- **new generation**: an earlier record changed, or the file shrank. The older generation stays readable.
- **unchanged**: nothing to keep. A file whose size and modification time are those of its last capture is not even read, while the catalog holds that capture's stamp; a file written within two seconds before a run looked at it is read again by the next run.
- **refused**: the file is named on stderr with its reason, and the run goes on.

A file that left the folder keeps its entry.

### Opaque strings

Before a record is stored, every JSON string value in it of 256 or more base64 or base64url characters, after an optional `data:` prefix (`data:<type>/<subtype>;base64,`, with `;<name>=<value>` parameters or no media type), becomes a placeholder that keeps its length and an XXH3 digest of the string: `[opaque: 41234 chars, xxh3 9f86d081884c7d65]`, or `[opaque image/png: 41234 chars, xxh3 9f86d081884c7d65]` for a data URI. Padding does not count, and a string of hex digits only, such as a commit SHA, is kept whatever its length. Object keys are kept as they are, whatever their length. The line keeps its place, so record numbers hold. A record that is not JSON is kept as it is. Raw bytes count the input; stored bytes count what the store grew by.

### Follows

`--follow` keeps the target now, then saves it and the kind that ran as a `[[follow]]` table at the end of the config file, leaving the rest of the file byte for byte. A target followed already is not saved again. While the file declares no follow, the defaults below apply; a declared follow replaces them, so the first `--follow` writes the defaults first.

A config file that is a symbolic link, or that this user cannot write (read-only, or another account's), as a Nix-managed one is, is not written, nor is one that cannot be replaced: the run exits 2, prints the lines to add and leaves the file as it was. A file that declares `follow` as an inline array (`follow = [...]`) is not written either: the run exits 2 and prints the inline table to add to that array. When the file is saved but its folder cannot be synced, the run says so in a warning; the follow is saved. Two `--follow` runs at once take turns on `config.toml.lock` beside the file, and each reads the file again before it appends, so both follows are saved. See [Configuration](configuration.md#keys).

`bilbo remember` with no target keeps every follow in order, into one store. With no `[[follow]]` in the config file it follows `~/.claude/projects` and `~/.codex/sessions`, both as `session`. A follow whose folder is missing is skipped with one line on stderr, and one that claims nothing says so; neither is an error. A file or folder a follow cannot read is refused and counts in the totals, as under a target.

### Sealing

New records go to `open.seg` in the state folder. A run with a target or none seals `open.seg` into a segment of the store when it ends, if `open.seg` holds frames and has reached `store.seal_at` or its oldest frame is `store.seal_after` old (60 minutes by default); during a run, `store.seal_at` alone seals. `open.since` in the state folder records when `open.seg` got its oldest frame. A bare run opens the store and seals what is due even when no follow claims a file, so the timer's pass seals by age and rebuilds a lost catalog; a run whose target claims nothing does not open the store. See [Configuration](configuration.md#keys).

### Hooks

`--hook` reads the JSON a Claude Code or Codex hook gets on stdin and keeps the new lines of its `transcript_path`, with the kind of the follow whose folder holds it, or the one detected. The transcript must be a regular file once its links are followed: a folder or a device is refused. To stay short, a hook skips the work that can take long: it never seals, whatever `store.seal_at` and `store.seal_after` say; it never runs the follows for another bilbo, so a request it finds stays for the next run with a target or none; and it never rebuilds the catalog: when the cache was lost or no longer matches the store, it keeps nothing and says so in one line. None of this bounds its time. It reads its input to the end and assumes the harness closes it after the payload, and when only readers hold the store it waits for them, up to `store.hook_wait` (2 seconds by default): then it keeps nothing and names the run that does, the path quoted as one shell argument: `kept nothing: another bilbo is reading the store; bilbo remember <path> keeps these lines`. A path that holds a control character, or is not UTF-8, gets no command: the line says so instead. A transcript whose path is not UTF-8 is never kept: `remember` refuses it. What a hook did not keep of any other, the next run that reads the transcript keeps: a bare run whose follow holds it, or a run with the transcript, or a folder that holds it, as target. It prints nothing on stdout and exits 0 once it runs, even on a payload it cannot read, a missing transcript or a config error; what went wrong is one line on stderr. Wrong arguments, such as a target beside `--hook`, are a usage error (exit 2) before the hook runs. Until the migration, set `paths.root`, `paths.state` and `paths.cache` in the config file first, as for the timer below. The `Stop` hook runs it at the end of every turn. Add it to Claude Code's settings or Codex's `hooks.json`:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "command -v bilbo >/dev/null 2>&1 || exit 0; bilbo remember --hook; exit 0",
            "timeout": 5
          }
        ]
      }
    ]
  }
}
```

### Two runs at once

One bilbo writes the store at a time: the writer holds `store.lock` and `store.writer` in the state folder, and a reader holds `store.lock` shared. A run with no target, or with `--hook`, that finds another bilbo writing sets the again flag (`store.again`) and exits 0 at once; one that finds only readers waits for them, since a reader never runs the follows again: a run with no target for as long as it takes, a `--hook` run for `store.hook_wait` at most. When a writer took the store from the readers during that wait, the `--hook` run sets the again flag for it, as for any writer. Before a run with a target or none releases the store, it runs every follow once more for each request it finds. A `--hook` run leaves a request where it is, for the next such run, since it cannot promise a pass over every follow within its time limit. A run with a target waits for the store instead, and says so on stderr, since a rerun of the follows would not keep its target.

### The timer

A timer runs `bilbo remember` with no target every 10 minutes. Nothing installs it yet: its files are in [`docs/reference/timer/`](timer/), with `@BILBO@` for the absolute path of `bilbo` and `@LOG@` for the log file. Until the migration, set `paths.root`, `paths.state` and `paths.cache` in the config file, since the timer sets no variable and the default folders are bilbo 0.20's (see [Configuration](configuration.md#folders)). Run these from a checkout of the bilbo repository: the unit files are in it, not in an installed bilbo.

On macOS, with launchd:

```sh
mkdir -p ~/Library/LaunchAgents ~/Library/Logs
sed -e "s|@BILBO@|$(command -v bilbo)|" -e "s|@LOG@|$HOME/Library/Logs/bilbo-remember.log|" \
  docs/reference/timer/io.github.delucca.bilbo.remember.plist \
  > ~/Library/LaunchAgents/io.github.delucca.bilbo.remember.plist
launchctl bootstrap "gui/$(id -u)" ~/Library/LaunchAgents/io.github.delucca.bilbo.remember.plist
```

On Linux, with systemd:

```sh
mkdir -p ~/.config/systemd/user ~/.cache
sed -e "s|@BILBO@|$(command -v bilbo)|" -e "s|@LOG@|$HOME/.cache/bilbo-remember.log|" \
  docs/reference/timer/bilbo-remember.service > ~/.config/systemd/user/bilbo-remember.service
cp docs/reference/timer/bilbo-remember.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now bilbo-remember.timer
```

### Output

Standard output, two lines:

```text
new <n>, appended <n>, new generation <n>, unchanged <n>, refused <n>
records <n>, raw bytes <n>, stored bytes <n>
```

Raw bytes are the input bytes kept; stored bytes are what the store grew by. A run with no target adds up every follow. On a terminal the same counts come as a marked message. A run that also ran the follows again for another bilbo says what that pass kept in one line on stderr. `--hook` prints nothing on stdout. A bare run that finds another bilbo writing the store prints nothing on stdout either; its planning lines, if any, come before the busy line on stderr.

A second `remember` with a target waits for the first one to finish writing the store, and says so on stderr.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when another bilbo holds the store and a run with no target asked it to run again; `--hook` once it runs, whatever happens |
| 1 | a file was refused, nothing was claimed under a target, a follow cannot run (also in a pass run again for another bilbo), or the target or the store could not be read |
| 2 | usage or config error (wrong arguments to `--hook` among them, before it runs), a store root that holds bilbo 0.20's `notes/` or `library/`, a state or cache folder inside the root, or a config file `--follow` cannot write |

## read

```text
bilbo read <ref> [--budget <tokens>]
bilbo read <ref> --outline [--budget <tokens>]
```

Prints an entry that `bilbo remember` kept: its metadata and its outline, or its records a page at a time. Each page ends with the command that prints the next one, so a reader that follows those lines sees every record once.

### The ref

```text
<ref>     = <entry> | <entry>#<locator>
<entry>   = <id> | bilbo:<id> | <session id>
<locator> = r<a> | r<a>-<b> | g<n> | g<n>:r<a> | g<n>:r<a>-<b>
```

- `<id>` is the entry's id, 26 characters, with or without the `bilbo:` a citation gives it.
- `<session id>` is the UUID a harness gave a session, or its first 8 characters or more: `bilbo read 2d6b1857`. It names the entry kept from the `.jsonl` file whose name ends in that UUID: Claude Code's `<uuid>.jsonl`, Codex's `rollout-<time>-<uuid>.jsonl`. A note named for a UUID is not a session: read it by its id. Upper or lower case. When it names several entries (one session kept from two folders, or a short prefix two sessions share), the run exits 2 and lists up to `read.matches_listed` of them (five by default) with their origins: give the id of one. A subagent has no UUID of its own: read it by its id, which its session's records name.
- `r<a>-<b>` is the records from `a` to `b`, counted from 1; `r<a>` is one record. A record outside the entry is refused, exit 1, with the number of records the entry has.
- `g<n>` is a generation: an entry whose file was rewritten keeps the old records as an older generation. With no `g`, the newest. A page of an older generation shows it in every locator (`#g1:r5`); a page of the newest shows the short form (`#r5`). The `more:` and `cut:` commands of the last line always name the generation that was read, so a file rewritten between two pages cannot make the next page read other records.

### The outline

With no record range, `read` prints what it knows of the entry, then one line per prompt of a session or per heading section of a note, each after the locator that reads it:

```text
bilbo:01M6ZGE4KTBYYH07AS2KVMSCAF
kind: session
origin: file:/home/me/.claude/projects/-home-me-src-garden/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl
captured: 2026-11-09 03:12 UTC
started: 2026-11-07 08:01 UTC
parent: bilbo:01M6ZFZ0V5T2K1X8Q4W7N9R3HB
records: 23
generation: 1
outline:
#r3 user: Thin the carrot rows, then tell me how many seedlings are left.
#r17 compact summary: This session is being continued. The carrots in the north bed were thinned from 42…
#r18 user: Here is the bed. What is the weed in the corner?
end: #r1-23 of 23 records
```

- `origin` is left out for an entry that has none, `started` for one whose records carry no time (a note), and `parent` unless the entry belongs to another: a subagent names its session.
- `generation` is `1 of 2` when the one shown is not the newest.
- A session lists its user prompts, its compact summaries and, for Codex, the messages one agent sent another. A note lists its frontmatter, the text before its first heading, and each heading with the headings above it: `#r28-31 The garden log > Watering > Tools`.
- An outline line is the prompt's first line, at most `read.outline_width` characters (100 by default); a longer one is cut to fit and ends in `…`. Inside a note's path, a heading over `markdown.heading_width` characters (100 by default) is cut to fit and ends in `…` too.
- A long outline is paged by the budget like the records, its metadata counted in: it ends with `more: bilbo read <id>#g<n>:r<a>-<b> --outline`. `--outline` with a range lists the passages that start in that range, without the metadata.

### A page

With a range, `read` prints each record that renders, after its locator:

```text
#r5 assistant: I'll count the rows first.
tool_use Bash:
command: wc -l beds/north.csv
#r6 tool_result Bash:
42 beds/north.csv
more: bilbo read 01M6ZGE4KTBYYH07AS2KVMSCAF#g1:r8-23
```

- A session record is one message: `user:`, `assistant:`, `tool_use <name>:` with the call's input a line per key, `tool_result <name>:` or `tool_error <name>:` with what came back, `compact summary:`. Thinking, the harness's own injections (Claude Code's `isMeta` entries, Codex's developer messages, its `AGENTS.md` and environment messages), entries of another thread and every bookkeeping line render nothing: they are never printed and never counted as missing. The thread of a Claude Code file is that of its first entry that renders: a session's file leaves out the entries marked `isSidechain`, and a subagent's file, where every entry is marked, reads as its own thread. An older Codex rollout does not say which user items the harness wrote; there an item is left out when its text starts with `# AGENTS.md instructions`, `<environment_context>`, `<recommended_plugins>` or `<skill>`, also when a person typed it. A rollout of Codex's first format, whose lines are bare items with no `payload`, reads as an empty outline. A range that holds none of the records that render prints its last line alone and exits 0.
- A note's records are its lines. They are printed a section at a time under the locator of the lines printed: `#r28-31`. A range that starts or ends inside a section prints the lines it holds. Headings are found by scanning lines, not by parsing Markdown's containers: inside or right after a list, a quote or indented code a section can start at the wrong line or carry the wrong label, but every line is printed, once.
- A pipe or a file gets each record's text as it is. A terminal gets its control characters escaped (`\u{1b}`), also when an agent runs bilbo there; a person's terminal also gets the locators in bold and the commands in colour.

The last line of every output is bilbo's, on a line of its own also when the last record has no line end, and nothing else is: a record prints as it was kept, so a line above the last can look like a locator or like any line of bilbo's. It is one of two:

| Line | Meaning |
|---|---|
| `more: bilbo read <id>#g<n>:r<a>-<b>` | The command for the next page: the rest of what was asked, from the next record that renders, in the generation this page read. It carries `--outline` after an outline page and `--budget <n>` when the flag gave the budget. |
| `end: #r<a>-<b> of <n> records` | Nothing is left of the range this run was asked for; the entry has `n` records. An entry with no record ends with `end: 0 records`. |

When the page ends in a record that was cut, the same line says so first: `cut: <n> more characters; bilbo read <id>#g<n>:r<k> --budget <b>; more: ...`, or `...; end: ...` when nothing is left after that record.

### The budget

A page holds as many whole records as fit `read.budget` tokens (see [Configuration](configuration.md#keys)); `--budget <tokens>` sets it for one run. A token is counted as 4 bytes until bilbo's tokenizer arrives, so the default of 7,000 is at most 28,000 bytes of records: under the 30,000 characters at which Claude Code cuts a command's output. That bound holds for a pipe or a file, which is what a harness reads. On a terminal each control character of a record prints as its six-character escape, which the budget does not count, so a page can be longer there.

A record is never split across pages. One that is over the budget by itself is printed alone, up to the budget, then the last line says how much is left and gives the command that prints it whole, before the command for the next page:

```text
$ bilbo read 01M6ZGE4KTBYYH07AS2KVMSCAF#r21-23 --budget 256
#r21 tool_result Read:
row 001 of the seed log: sown, watered, thinned
row 002 of the seed log: sown, watered, thinned
[rows 003 to 020 are left out of this example]
row 021 of the seed log: sown, wat
cut: 1885 more characters; bilbo read 01M6ZGE4KTBYYH07AS2KVMSCAF#g1:r21 --budget 1000; more: bilbo read 01M6ZGE4KTBYYH07AS2KVMSCAF#g1:r22-23 --budget 256
```

The cut falls between two characters. The budget of the `cut:` command is what the whole record counts under the locator of that command, rounded up to the next thousand; it is always one `--budget` accepts, and it prints the record whole also after the file was rewritten. The cut record counts as printed: `more:` goes on after it. A long section of a note is not cut: it goes on at its next line on the next page. Only a single line over the budget is cut.

### Reading while bilbo writes

`read` only reads: it creates and changes nothing in the store, the state or the cache folder. It holds the store's lock in shared mode for as long as it takes to fetch the entry's records, and lets it go before it renders or prints. While a `bilbo remember` holds the store, `read` waits up to `store.wait` (10 seconds by default), says so on stderr once, and then gives up with exit 1: `another bilbo is writing the store`. Run it again.

`read` uses the catalog in the cache folder when it holds every frame of the store. When the catalog is missing or behind (the cache folder was deleted, say), `read` walks every segment instead, under the lock, which takes as long as the store is large: reads are slow until the next `bilbo remember` with a target or none rebuilds the catalog. A `bilbo remember --hook` that finds a reader on the store waits for it up to `store.hook_wait` (2 seconds by default), then leaves its lines to the next run and says so; a run with a target or none waits for the reader.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when no record of the range renders |
| 1 | no entry has the id or the session id, the range or the generation is outside the entry, the entry is of a kind this bilbo cannot read, there is no store, the store is being written for longer than `store.wait`, or the store could not be read |
| 2 | usage or config error: a ref that does not parse, a session id that names several entries, a `--budget` under 256 |
