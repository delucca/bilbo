# Commands

## Every verb

Results go to standard output; diagnostics go to standard error, each line prefixed `bilbo: ` off a terminal.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | refused, problems found or nothing matched |
| 2 | usage or config error |

`bilbo digest` is the one verb that always exits 0 ([digest](#digest)).

## config

```text
bilbo config [<key>]
bilbo config set <key> <value>
```

Shows every setting: its key, its value and where the value came from. With a key, it prints that key's value alone, for scripts. `bilbo config set <key> <value>` writes one key into the config file and leaves every other byte as it was; the value is one TOML value, and a text that is not TOML is taken as text (`30m`, `anthropic`). A value its key refuses is said in the key's words and nothing is written. When it does not write it says why, prints the key with its whole path and the value, and exits 2; putting that line in the file is yours. It runs even when the file holds a value bilbo refuses, and says so after it wrote: a file with two bad values is mended one key at a time. It takes turns with other writes of the file on the lock beside it. Otherwise edit the config file. [Configuration](configuration.md) lists the keys, their variables and their defaults.

### Output

Standard output, one line per key, in the order [Configuration](configuration.md#keys) lists them:

```text
paths.root = "/home/me/.local/share/bilbo" (default)
paths.state = "/home/me/.local/state/bilbo" (default)
paths.cache = "/home/me/.cache/bilbo" (default)
paths.temp = "/tmp" (default)
store.seal_at = "16 MiB" (/home/me/.config/bilbo/config.toml:5)
store.seal_after = "60m" (default)
store.compression = 3 (BILBO_STORE_COMPRESSION)
store.tear_budget = 4 (default)
store.wait = "10s" (default)
follow = [{ path = "/home/me/.claude/projects", kind = "session" }, { path = "/home/me/.codex/sessions", kind = "session" }] (default)
remember.editor = "" (default)
read.budget = 7000 (default)
```

The value is TOML, as the config file would hold it. The origin is `default`, the config file and the line that set the key, the variable, or the flag. On a terminal the values line up and the origins are dimmed.

`bilbo config store.seal_at` prints `16 MiB`: a string without its quotes, any other value as TOML. A key bilbo does not know is a usage error.

A config file or variable that bilbo refuses fails `bilbo config` as it fails every other verb, with the same message, except the 0.20-root refusal and the refusal of a state or cache folder inside the store root, which a verb runs when it opens the store, and `bilbo digest` when it checks the store's folders before that (`the state folder <path> is inside the store root <root>, which holds only segments; set paths.state to a folder outside the root`). `bilbo config` runs neither. So `bilbo config` checks a file you edited.

### set

`bilbo config set <key> <value>` writes only when the file read again is the old one with that key set. The value is one TOML value; a text that is not TOML is taken as text, and so is a text the key takes only as a text (`bilbo config set llm.anthropic.api.version 2026-10-21` writes `"2026-10-21"`). A byte-order mark and a file's own line end are kept. When it does not write, the file is as it was and no temporary is left in its folder, except that the lock's file, `<config>.lock`, stays there once it was made; it exits 2 with this on standard error:

```text
bilbo: cannot set <key> in <file>: <why>; set it by hand:
```

and the key with its whole path and the value alone on standard output (`derive.enabled = true`): the line is what the file must come to say, and where it goes is yours. `<why>` is one of these:

| Reason | `<why>` |
|---|---|
| The file is a link | `it is a symbolic link` |
| The file is no regular file | `it is not a regular file` |
| The file cannot be written | `it is read-only` |
| The lock's folder or file cannot be made, the lock cannot be taken, the file cannot be read again, the temporary cannot be made, or the rename fails | the system's error |
| The file holds the key as a dotted key | `line <n> sets it with a dotted key` |
| The file makes the key's table with a dotted key | `line <n> sets its table with a dotted key` |
| The file holds the key inside braces | `line <n> sets it inside braces` |
| The file makes the key's table inside braces | `line <n> sets its table inside braces` |
| The file holds the key as a table of its own | `line <n> sets it as a table of its own` |
| The new text does not parse or would change another key | `bilbo could not place it` |

The line number is the first line of what the clause names. A file that did not load before the edit is said as `cannot set <key>: <the error as load says it>; set it by hand:`. After a write that lands, a variable that sets the same key wins over the file and a warning says so, without its value: `<VARIABLE> is set and wins over <key> in <file>`.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | refused, problems found or nothing matched |
| 2 | usage or config error |

## remember

```text
bilbo remember <file|folder> [--kind <kind>] [--tag <tag>]... [--follow]
bilbo remember - [--tag <tag>]...
bilbo remember --edit [--tag <tag>]...
bilbo remember
```

Keeps files as entries in the store. An entry is a sequence of records, numbered from 1 per generation. Run it again on a file or folder and it keeps only what is new; a note with no file is a new entry each time. It keeps the file or folder it is given; [`bilbo refresh`](#refresh) keeps every follow. A lone `-` keeps a note from standard input, and `--edit` one written in an editor ([A note with no file](#a-note-with-no-file)). With no argument, on a terminal, it asks what to keep ([The prompt](#the-prompt)).

### Kinds

A kind picks the files under the target, splits each into records and proposes the entry's meta. `bilbo remember --help` lists the kinds.

| Kind | Claims | Records |
|---|---|---|
| `markdown` | `.md` files | one per line, newline included; a final line without a newline is one record |
| `session` | files whose name ends in a UUID and `.jsonl` (Claude Code's `<uuid>.jsonl`, Codex's `rollout-<time>-<uuid>.jsonl`), and every `agent-*.jsonl` under Claude Code's `<uuid>/subagents/`, workflow runs' included; each also when packed as `<name>.jsonl.zst`, as Codex packs its older rollouts | one per complete line, newline included; a line still being written waits for a later run |

A `markdown` file that is not UTF-8 is refused. A valid ULID in the frontmatter `id` becomes the entry's id at its first capture; a file with no usable `id` gets a fresh one. A file whose `id` an entry holds under another capture label is that entry when no file at that label still carries the `id`: the entry moves to this file's capture label, the file is kept as any change to it is, and stderr says `moved <path>: its id <id> was held by file:<old path>`. An `id` a forgotten entry holds is another case ([A forgotten file](#a-forgotten-file)). While a file at the old label still carries the `id`, this one is refused: two files with one `id`. So is a file whose own path was already kept as another entry: `refused <path>: its id <id> is held by <label>, but this path was kept as entry <id>`. A renamed note whose `id` another entry than the one kept at that path holds cannot take the name of a note that was deleted, and two such notes cannot swap names: give the file another name, or free the name first with [`bilbo forget`](#forget). A file that keeps its size and its modification time is not read again, so two notes that swapped names are seen only once one of them changes. A file with no usable `id` that moved is a new entry.

A `session` folder walk skips Claude Code's `memory/`, `workflows/` and `tool-results/` folders (a project's memories, a session's workflow definitions and its large tool outputs), but nothing below a `subagents/` folder, where `workflows/wf_*/` holds the subagents of workflow runs. It never claims the `.meta.json` beside a subagent, and never reads it: it names no session. Main sessions are kept before subagents, so a subagent's entry names its session's entry as its id label: the session `<uuid>.jsonl` in the folder that holds `<uuid>/subagents/`, however deep below it the subagent sits. A subagent kept before its session, as a target of its own, gets its id label the next time a run actually reads it. Growth is one cause of a read, not the only one. A run skips a file only while the catalog keeps a trustworthy stamp for it, and a stamp taken within two seconds of the file's last write is not kept. Which harness wrote a file, Claude Code, Codex or Cursor (whose Claude-shaped lines carry `role` and no `type`), is read from its lines each time and never stored. A file that is not UTF-8, or whose lines are none of these, is refused.

### Detection

Without `--kind`, every kind says which files it claims under the target. Exactly one kind claims files: it runs. None: there is nothing to keep (exit 1). Several: a usage error that names them and asks for `--kind`. With `--kind`, only that kind runs; an unknown kind is a usage error. `~/.claude/projects` holds `memory/*.md` files, so it needs `--kind session`.

### What the walk skips

A folder is walked recursively. Hidden entries (names starting with `.`) and symbolic links that the walk finds are skipped, and the walk never follows a link, so it cannot loop.

A target named on the command line is taken as given, even when it is a symbolic link or a hidden name, as `find -H` and `git add` treat their arguments. Its capture label is the resolved path, and a kind tests the resolved file, not the name given: a `note.md` link to a file without `.md` keeps nothing.

### Outcomes

An entry is found again by its capture label (`file:<absolute path>`): the path its file was last kept under. For each file:

- **new**: no entry has this capture label yet, and none holds the file's `id`. A file bilbo takes for a forgotten entry's is another case ([A forgotten file](#a-forgotten-file)).
- **appended**: the earlier bytes are unchanged and the file gained records; only those are kept.
- **new generation**: an earlier record changed, the file shrank, or the file is gone from a follow ([A file that is gone](#a-file-that-is-gone)). The older generation stays readable.
- **unchanged**: nothing to keep. A file that was only renamed is unchanged too: the run writes its move alone, so `stored bytes` is over 0. A file whose size and modification time are those of its last capture is not even read, while the catalog holds that capture's stamp; a file written within two seconds before a run looked at it is read again by the next run. A file a run passes over for a forgotten entry is unchanged too ([A forgotten file](#a-forgotten-file)).
- **refused**: the file is named on stderr with its reason, and the run goes on.

### A file that is gone

Nothing is ever taken out of the store: an entry outlives its file, and `bilbo read` reads it. What a gone file does to `recall` depends on the kind, and only a pass over the follows looks: [`bilbo refresh`](#refresh), or the pass another bilbo runs for it.

- A `session` file that is gone changes nothing. Harnesses delete old sessions, and keeping them is what bilbo is for: the entry stays as it was, in `recall` too.
- A `markdown` file that is gone from a followed folder withdraws its entry: the entry gets a new generation with no record, so `recall` no longer gives that entry once the run has brought the index up to date. stderr says `withdrew <path>: its file is gone; bilbo read <id>#g<n> prints the outline of what it held`, and the entry counts in `new generation`. `bilbo read <id>#g<n>:r<a>-<b>` prints its records. When the file is back with text, the next run keeps it into the same entry, as `appended`. For an entry forgotten since, see [A forgotten file](#a-forgotten-file).

The pass keeps every follow's files first and looks for gone files after, so a note that was renamed or moved to another followed folder is moved, not withdrawn. A note whose file was refused in that pass because its new name was kept as another entry is not withdrawn either: its file is not gone. A follow withdraws only when it kept or found unchanged at least one of its files in that pass: a follow whose folder is missing, or shows no file, or whose files all vanished, withdraws nothing. So a folder that was emptied, or a disk that is not mounted, takes no note out of `recall`, and the last note a follow shows stays in `recall` when it is deleted, unless a follow above it still shows a file. [`bilbo forget`](#forget) takes such a note out of `recall`. Only a path the system says is not there is gone: one that cannot be looked at is left as it is. A follow of one file never withdraws.

Each follow decides alone, for every entry kept under its folder: a follow above another one withdraws under it too. A subfolder that is a mount point and is not mounted looks like an empty folder, so the notes under it leave `recall` until it is back. A note is gone under its old path too when a file still answers there under another: on a volume that folds case, after its name or a folder above it changed only in case; or after a rename to a name its kind does not claim (`n.md` to `n.MD`). One limit: when the followed folder itself is renamed only in case, that follow no longer names the entries kept under its old spelling, as after any moved folder: a note with an `id` moves, and a note with none is kept again and stays twice, unless a follow above the folder still shows a file and withdraws the older entry.

A run with a target withdraws nothing, and `bilbo remember <a file that is gone>` is an error.

### A note with no file

```text
bilbo remember - [--tag <tag>]...
bilbo remember --edit [--tag <tag>]...
```

`bilbo remember -` keeps what it reads from standard input, to its end, as a note: an entry of the kind `note`, which [`recall`](#notes) lists apart from the chunks. Each call is a new entry: the same text piped twice is two notes. The text is kept as UTF-8, without the white space at its start and its end; a text with nothing left keeps nothing and exits 1. A note the store would keep only as a placeholder is refused: one that is nothing but encoded data, as the store judges it for any record (a long run of base64 characters, alone or after a `data:` head; a run of hex digits alone is kept). `--tag` works as for a file; `--kind` and `--follow` do not stand with a note. The `-` is taken as typed: a file named `-` is given as `./-`.

A run that keeps a note prints the two lines of [Output](#output) with `new 1`, and `kept the note as <id>` on stderr. `bilbo read <id>` prints the note; its capture label is `note:<id>`, until it is forgotten. The run brings the search index up to date as any `remember` does, so `recall` finds the note at once. `bilbo derive` writes no note about it. Where `derive.in` names it, a run may send its text to the provider for its summary, by the rules of [Summaries](#summaries): a note of fewer than `derive.summary.least` tokens gets no first one. `derive.not_in`, or `derive.summary.enabled = false`, keeps a note from any.

`bilbo remember --edit` opens an editor on an empty file and keeps what was saved, as `git commit` does for a message. The editor is the command of [`remember.editor`](configuration.md#keys), which `bilbo config remember.editor` prints. `/bin/sh` runs it with the file as its last argument; with no editor, the run exits 2. The file is `note.md` in a folder bilbo makes for it, yours and of mode 0700, `.bilbo-note-<id>` in the folder of [`paths.temp`](configuration.md#keys), which `bilbo config paths.temp` prints. The mode is what keeps others out: bilbo does not look at an access list a temp folder hands down to what is made in it. A file that is empty or holds only white space keeps nothing and exits 1. An editor that exits with another code than 0, or that a signal ends, keeps nothing, whatever was saved. An editor that ran and whose end bilbo could not learn keeps nothing either, and its file stays where the line says. The folder is removed, with all it holds, when the note was kept or there was no text. When there was text and the note was not kept, it stays and the run's line says where the file is, so what you wrote is not lost; it is yours to remove. When the store failed at the write of the note itself, the line also gives the id the note may be under; a store that failed before that wrote nothing, and its line gives no id. An editor that failed is said first, whatever its file holds. The store is opened only after the editor is done, so no other bilbo waits while you write.

A note kept this way cannot be changed: [`bilbo forget <id>`](#forget) takes it out of `recall`. A note you will want to edit is a Markdown file in a followed folder, which `remember` and `refresh` keep up with.

### The prompt

`bilbo remember` with no argument asks what to keep when a person is at the terminal: standard input and stderr are terminals, and no agent runs bilbo. It offers a note typed there (ended with Ctrl-D on a line of its own), a note written in the editor, and a file or folder (a leading `~/` is your home folder), then does what `bilbo remember -`, `--edit` or `<file|folder>` does. An empty answer keeps nothing and exits 0, as does the end of input at a question. Anywhere else a bare `bilbo remember` is a usage error: it never runs the follows, which is [`bilbo refresh`](#refresh).

### Packed sessions

Codex packs a rollout it has not written for a week into `<name>.jsonl.zst`, and unpacks it when the session is resumed. Both files are one entry, kept under the capture label of the plain file (`file:<folder>/<name>.jsonl`), also while only the packed file is on disk. A run reads the plain file when it is there and the packed one otherwise, unpacked in memory; it writes neither. A rollout packed after it was kept is unchanged; one unpacked and grown is appended. A rollout named or followed as one file is found under its other form when Codex has packed or unpacked it since. A packed file that does not unpack is refused. So is one made to unpack to far more than it takes on disk: its unpacked bytes, with 64 more counted for each line, may come to `session.unpack_floor` (128 MiB by default), or to `session.unpack_ratio` (256 by default) times the packed file's size when that is more. A session packs to a fraction of its size; a file that unpacks to thousands of times its size was made to. That count is not all a run holds: to keep a session, packed or plain, it holds the bytes twice, some forty bytes more for each line and what it packs them to, and unpacking takes a window of its own, no larger than that limit rounded up to a power of two, from 8 MiB to `session.unpack_window` (128 MiB by default). A file refused under one setting is tried again by the next run. A plain file has no such limit. A refusal names the file that was read or, when none could be, the plain file's name. A rollout moved to another followed folder, as Codex's archive does, has another capture label there and becomes another entry.

### Opaque strings

Before a record is stored, every JSON string value in it of 256 or more base64 or base64url characters, after an optional `data:` prefix (`data:<type>/<subtype>;base64,`, with `;<name>=<value>` parameters or no media type), becomes a placeholder that keeps its length and an XXH3 digest of the string: `[opaque: 41234 chars, xxh3 9f86d081884c7d65]`, or `[opaque image/png: 41234 chars, xxh3 9f86d081884c7d65]` for a data URI. Padding does not count, and a string of hex digits only, such as a commit SHA, is kept whatever its length. Object keys are kept as they are, whatever their length. The line keeps its place, so record numbers hold. A record that is not JSON is kept as it is. Raw bytes count the input; stored bytes count what the store grew by.

### Follows

`--follow` keeps the target now, then saves it and the kind that ran as a `[[follow]]` table at the end of the config file, leaving the rest of the file byte for byte. A target followed already is not saved again. While the file declares no follow, the defaults of [`follow`](configuration.md#keys) apply; a declared follow replaces them, so the first `--follow` writes the defaults first.

A config file that is a symbolic link, or that this user cannot write (read-only, or another account's), as a Nix-managed one is, is not written, nor is one that cannot be replaced: the run exits 2, prints the lines to add and leaves the file as it was. A file that declares `follow` as an inline array (`follow = [...]`) is not written either: the run exits 2 and prints the inline table to add to that array. When the file is saved but its folder cannot be synced, the run says so in a warning; the follow is saved. Two `--follow` runs at once take turns on `config.toml.lock` beside the file, and each reads the file again before it appends, so both follows are saved. See [Configuration](configuration.md#keys).

### Tags

`--tag <tag>` declares a tag for every entry the run keeps under the target: a new entry is kept with it, and an entry kept before gets it added, whether or not its file changed. A run never takes a tag away. A tag is parts of the letters `a` to `z` and digits joined by single hyphens, at most 64 bytes: `rust`, `carpentry-docs`. A tag an entry does not carry yet is refused when it would leave the entry with more than `tag.cap` tags, 32 unless set: the file is refused, as any refused file is, before anything of it is kept, and the files kept before it stay kept. A renamed file refused for the cap has moved all the same: its entry is under the new path, and stderr says so. A tag the entry already carries is never refused, whatever the number of its tags. `--tag` stands with a file or folder and with a note, from `-` or from `--edit`: the note is kept with the tag, as a new entry is. With nothing to keep it is a usage error, and it does not stand with `--follow`: a follow keeps no tag. Standard output does not change. With `--tag`, stderr says `tagged <n> entries`, the entries whose tags the run changed, when there is one or more. A tag no entry carried before the run is said too: `tag:<name> is new`.

```text
bilbo remember ~/library/rust --tag rust
```

### The index

A run also brings the search index to what the store holds, inside the lock that kept the files and before it releases: it indexes the entries that changed since the cursor the index's last commit recorded, and an entry whose kind cannot promise that appending records changes no passage already indexed (Markdown) is indexed again whole. An entry that moved is indexed again whole too, since its labels changed. When the index cannot be continued, because it was never made or is broken, `search.chunk_tokens`, `search.chunk_overlap`, `markdown.heading_width` or a rule of this bilbo changed, or the catalog was rebuilt, the run makes it again in full, as [`reindex`](#reindex) does, says so on stderr before it starts (`building the search index: <n> entries`, or `rebuilding the search index: <why>`) and can take minutes on a large store. The line comes when the build names more entries than the run has kept, so a first run on a new store, where the entries it kept are all there is to build, says nothing, and neither do two other long builds: a store whose index was lost and whose every entry is captured again in the same run, and a first run that keeps one very large entry. A file of the index that is cut short, or that the index library panics on when it opens it, makes the index broken (the library's own panic text can appear on stderr while the run looks at it), and the run makes it again by itself. A run that finds nothing new does not even load the tokenizer. A run with a target that claims nothing does not open the store, so it does not touch the index.

If the index cannot be read or written, or its folder is not bilbo's, the run prints `the search index was not updated: <error>` and exits 1, and when the index itself failed (not a folder that is not bilbo's, which nothing repairs) the line ends with `; bilbo reindex makes the index again`; damage that shows only when the index's columns are read, which the check of its files does not see, fails the update with the same line (`its content could not be read`, after tantivy's own panic text on stderr), and `bilbo reindex` repairs it; what it kept stays kept, and the run tries the index no more. Nothing is remembered between runs: the next run plans again, and a rebuild that fails late costs its whole time again at every run, up to about five minutes of the store held every ten on a large one, until what makes it fail is fixed. An entry the reader or the tokenizer fails on is named on stderr and exits 1; it is tried again by the next two runs, and then waits for a change to it or for `bilbo reindex`.

An index kept current by `remember` holds the same documents as one `reindex` builds and ranks them as it does.

### Sealing

New records go to `open.seg` in the state folder. A run seals `open.seg` into a segment of the store when it ends, if `open.seg` holds frames and has reached `store.seal_at` or its oldest frame is `store.seal_after` old (60 minutes by default); during a run, `store.seal_at` alone seals. The first run that writes a store whose `open.seg` an earlier bilbo wrote (segment format 1) seals that `open.seg` at once if it holds frames, whatever its size or age, and writes format 2 from then on. `open.since` in the state folder records when `open.seg` got its oldest frame. `bilbo refresh` opens the store and seals what is due even when no follow claims a file, so the timer's pass seals by age and rebuilds a lost catalog; a `remember` whose target claims nothing does not open the store. See [Configuration](configuration.md#keys).

### Two runs at once

One bilbo writes the store at a time: the writer holds `store.lock` and `store.writer` in the state folder, and a reader holds `store.lock` shared. A `bilbo refresh` that finds another bilbo writing sets the again flag (`store.again`), prints `another bilbo is writing the store; requested another pass over the follows` and exits 0 at once; one that finds only readers waits for them, since a reader never runs the follows again. A bilbo that writes the store takes the request when it reaches the end of its work: before it releases the store, it runs every follow once more for each request it finds, and brings the index up to what each pass kept. A `remember` waits for the store instead, and says so on stderr, since a pass over the follows would not keep what it was given.

### Output

Standard output, two lines:

```text
new <n>, appended <n>, new generation <n>, unchanged <n>, refused <n>
records <n>, raw bytes <n>, stored bytes <n>
```

Raw bytes are the input bytes kept; stored bytes are what the store grew by. On a terminal the same counts come as a marked message. A run that also ran the follows again for another bilbo says what that pass kept in one line on stderr. The index adds lines on stderr only when it rebuilds, leaves an entry out or fails; stdout does not change. An empty answer at [the prompt](#the-prompt) prints nothing on standard output: it keeps nothing, says `nothing kept` on stderr and exits 0.

A second `remember` with a target waits for the first one to finish writing the store, and says so on stderr.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also an empty answer at the prompt, which keeps nothing |
| 1 | a file was refused, nothing was claimed under the target, a note was refused or empty, the editor failed, standard input or an answer at the prompt could not be read, a pass run again for another bilbo refused a file or could not run a follow, the search index could not be updated or an entry could not be indexed, or the target or the store could not be read |
| 2 | usage or config error, `--edit` with no editor named, a store root that holds bilbo 0.20's `notes/` or `library/`, a state or cache folder inside the root, or a config file `--follow` cannot write |

## refresh

```text
bilbo refresh
```

Keeps every follow, in order, into one store: what each followed folder or file gained or changed since the last run, as [`bilbo remember`](#remember) keeps a target. With no `[[follow]]` in the config file it keeps the default follows: [`follow`](configuration.md#keys) lists them, and `bilbo config follow` prints the ones a run keeps. A follow whose folder or file is missing is skipped with one line on stderr, and one that claims nothing says so; neither is an error. A file or folder a follow cannot read is refused and counts in the totals, as under a target. `bilbo remember <target> --follow` declares a follow ([Follows](#follows)). A Markdown file that is gone from a followed folder withdraws its entry ([A file that is gone](#a-file-that-is-gone)). A file it takes for a forgotten entry's is passed over ([A forgotten file](#a-forgotten-file)).

It opens the store even when no follow claims a file, so it seals by age and rebuilds a lost catalog ([Sealing](#sealing)), and it brings the search index up to date as `remember` does ([The index](#the-index)). When another bilbo is writing the store it does not wait: it asks that bilbo for one more pass over the follows and exits 0 ([Two runs at once](#two-runs-at-once)).

### The timer

A timer runs `bilbo refresh` every 10 minutes. Nothing installs it yet: its files are in [`docs/reference/timer/`](timer/), with `@BILBO@` for the absolute path of `bilbo` and `@LOG@` for the log file. Until the migration, set `paths.root`, `paths.state` and `paths.cache` in the config file, since the timer sets no variable and the default folders are bilbo 0.20's (see [Configuration](configuration.md#folders)). Run these from a checkout of the bilbo repository: the unit files are in it, not in an installed bilbo.

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

The timer is what keeps a session as it grows: a turn is kept and indexed by the next run after it is written, at most 10 minutes later while runs are short. The 10 minutes are the interval between the timer's starts, not a bound: a run that rebuilds the index takes minutes, and a failing or suspended run takes longer. Without a timer, what the follows gained is kept only when `bilbo refresh` is run by hand, or `bilbo remember <target>` for one file or folder.

### Output

The two lines of [`remember`](#remember), added up over every follow. On a terminal the same counts come as a marked message. When another bilbo is writing the store it prints nothing on standard output; its planning lines, if any, come before the busy line on stderr.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when another bilbo holds the store and was asked for a pass |
| 1 | a file was refused, a follow cannot run, the search index could not be updated or an entry could not be indexed, or the store could not be read |
| 2 | usage or config error, a store root that holds bilbo 0.20's `notes/` or `library/`, or a state or cache folder inside the root |

## recall

```text
bilbo recall <query>... [--in <label>]... [--kind <kind>]...
               [--budget <tokens>] [--page <n>]
```

Searches everything bilbo keeps, or the entries `--in` names, and prints the chunks and the notes that match best, each with its text, read back from the store. It only reads: the one file it writes is the query log ([The query log](#the-query-log)).

### The query

The arguments are joined by one space: `bilbo recall how did we decide the index format` is one query. A query that starts with a dash needs `--` before it: `bilbo recall -- -x flag`. The query is cut into words as the index cut the chunks (folded, then lowercased, without the 33 English stopwords, stemmed), after the words of `recall.question_words` (`what`, `did`, `we`, `should`, `how` and `anything` by default) are taken out of it; they stay searchable in what the index holds. The fold comes before the lowercasing, in the query and in what the index keeps: accents and other marks are dropped, and a letter or digit in a compatibility form (`ﬁ`, a full-width letter, `²`) becomes the plain one when the plain one is all letters and digits, so `nao` finds `não` and `configuration` finds `conﬁguration`; a form whose plain one holds another sign stays as it is (`2¼` is the one word `2¼`), and a sign that is no letter or digit (`™`) stays and still ends a word; Hangul is searched by its jamo, so a syllable and its decomposed spelling are one word. A query bilbo reads as written in a language of `recall.languages` can also lose that language's function words ([Languages](configuration.md#languages); Portuguese's are the 207 words of NLTK's list), which count as typed or without accents, so `nao` is `não`. A language detector reads the query as it was typed; it is asked only when the query has two words or more and one of them is a function word of a listed language, and a word that is an English word too (`do`, `me` and `so` in Portuguese) is not enough to ask it. Its answer is not enough either: the words leave only when the language it answers is listed and the query holds a function word of that language that is not an English word too. A query of one word, as the analyzer cuts it, is not tested for a language at all, so `bilbo recall nas` finds a NAS; `recall.question_words` still applies to it. Like the question words, these stay searchable in what the index holds. The detector can be wrong, mostly on short queries: `recall.languages = []` turns this off. A query with no word left matches nothing. Chunks are ranked by BM25 with `recall.bm25_k1` and `recall.bm25_b`, from the exact length of each chunk and the statistics of every chunk in the index. Query words that stand in a row in a chunk add to its score ([Words in a row](#words-in-a-row)). Equal scores come in the order of entry id, generation, record and part. At most `recall.candidates` chunks are considered (400 by default); the rules of [Copies and long records](#copies-and-long-records) pass some of them over, and at most `recall.cap` hits are shown from one entry (no limit by default).

### Misspelt words

A query word no chunk of the index holds is taken for misspelt when it is made of the letters `a` to `z` only, after stemming, and has at least `recall.typo_letters` of them (4 by default) and at most `recall.typo_max_letters` (64 by default, the highest it goes; set below `recall.typo_letters`, no word is taken). The index's nearest terms are searched in its place: terms one edit away, or two when the word has `recall.typo_two_edits` letters or more (8 by default). An edit is a letter added, dropped or changed, or two neighbouring letters swapped. Of those terms the nearest `recall.typo_terms` are kept (3 by default), the nearest first and, among equals, the one more chunks hold; each weighs its share of the chunks the kept terms are in, so together they weigh as one word. A word some chunk of the index holds is never touched, whatever `--in` and `--kind` ask, and a word with no term near it is searched as it is. `recall.typo_terms = 0` turns this off.

The terms are the index's, so stems: `bilbo recall carots` searches `carrot`, which a Markdown file about carrots holds. stderr names each word that was read this way, once, and the terms searched in its place:

```text
bilbo: typos: carot as carrot, cart; pyton as python
```

Only a word no chunk holds is read this way: a misspelt word that some chunk holds is searched as typed and not expanded. The notes follow the chunks: a word some note holds is searched among the notes as typed, and a word no note holds is searched there as the terms chosen for the chunks, when there are any.

### Words in a row

Two query words that stand next to each other in a chunk, in the query's order, can add to its score. `recall.pair_weight` says how much: a number from 0 to 10, 0.25 by default. `recall.pair_weight = 0` turns it off: nothing is added and no position is read. Each two words in a row of the query make a pair, counted once for each place it stands in the query. The query's words are the ones it is searched by: the stopwords, the words of `recall.question_words` and the function words it loses by the rule of [The query](#the-query) are out of it first. A stopword is out of the chunks too, so `state of the art` is `state art` in the query and in the chunk, and the pair stands. So is a word of one letter (`e`, `a`, `o`), and a function word that is an English stopword as well (Portuguese `as`, `no`): `arroz e feijão` is `arroz feijao` in both. A question word, or a function word the chunk keeps (Portuguese `de`, `do`, `da`, `em`, `para`), stands between the two in the chunk: `banco de dados` is asked as the pair `banco dados`, a chunk that says `banco de dados` has `de` between the two and gets nothing from it, and a chunk that says `banco dados` does. A word twice in a row pairs with itself. A pair is scored as one word is, by the chunks where the second word stands right after the first: a pair few chunks hold adds more than one many hold, and a chunk that holds the words apart or the other way round gets nothing from it. A chunk's pair scores are added up, and the sum times `recall.pair_weight` joins its BM25 score. A word no chunk holds is in no pair, and its neighbours are not paired across it; a word read as misspelt is in none either. The bonus is never negative, reaches only chunks that hold both words, and is the same whatever `--in` and `--kind` ask. The index keeps where each word stands to answer this, so it is larger than one that does not.

### Kinds

`--kind <kind>` searches only entries of that kind; repeat it to search several. `note` is a kind too: `--kind note` prints the notes alone, and naming only other kinds prints no note. A kind no reader reads is a usage error, exit 2, naming the kinds. The filter changes no score: a chunk scores the same with it and without.

### A hit

Standard output is one hit after another, with no blank line between them:

```text
=== 2T6W9YB0DE#r3-7<TAB>4 lines
Session started 2026-11-07 08:01 UTC:
user: Thin the carrot rows.
...
```

The header is `=== `, the ref and the number of lines, apart by tabs. A chunk cut from a record too long for one chunk adds `part <k>/<m>`, the last hit of a run that was cut to the budget adds `cut`. A note's header adds its kind, `note`, after the number of lines; a chunk's has no kind. No header has a date. The ref is what `bilbo read` takes: the last 10 characters of the entry's id, then the records of the chunk, with no generation. When another entry's id ends with the same 10 characters (two Markdown files can be given such ids by their frontmatter), the ref holds the whole 26-character id instead, so `bilbo read` takes every ref a header prints, as `recall` read the store, and each names one entry. A parser takes a ref of either length. A ref is the short id when no other entry's id ended with it at that moment: if an entry kept later ends alike, `bilbo read` says so and lists the entries. When a hit is from: a session's hit opens with `Session started <time> UTC:`, `bilbo read <ref>` prints `captured:` and `started:` in its outline, and a Markdown file's hit shows no date. After the header come exactly as many lines as it says. A line ends at a line feed and nowhere else: a carriage return is content. A line feed ends the hit; nothing scans the text for a header, so a chunk that quotes an earlier `recall` cannot forge one. A tool takes a header, then `<n>` lines, and so on; after the last hit one more line can stand, which starts with `more: ` ([Pages](#pages)). It stands only where a header would, so no line of a hit's text is taken for it: a hit whose own last line reads like it is a hit, by its header's count. The text is the store's, with one line feed between the passages a chunk joins, which the store does not hold, after a line its kind adds: a session's hit opens with `Session started <time> UTC:` (the time of its first record, or `Session date unknown:`); a Markdown file's has none. On a terminal each control character of a line prints as its escape, tabs kept, and so do the twelve bidirectional controls (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069); plain output keeps them as they are.

### Notes

When the store holds notes, the answer is the chunks, then the notes. A note is a paragraph a model wrote about records of an entry (`bilbo derive`), or a text kept with no file ([`bilbo remember -`](#a-note-with-no-file)). The two lists are ranked apart: a note is never among the chunks. Each list is scored over its own documents: the chunks score exactly as they would in a store with no note, and a note is scored among the notes. So the chunks are the same hits, in the same order, with the same text, up to where the list ends: with no note to rank, the chunks take the notes' room too ([The budget](#the-budget)). One more thing can differ: a chunk's header gives its entry's id in full when another entry of the store ends in the same ten characters, and a note's entry counts there too, since `bilbo read` takes a ref against the whole store.

```text
=== 5N3K8QW1ZX#r4<TAB>2 lines<TAB>note
01M6ZGE4KTBYYH07AS2KVMSCAF#g1:r3-19 2026-11-07
The user asked to thin the carrot rows ...
```

The header's ref is the note itself. Its first line is what it was written from: the entry's whole id, the generation and the records, then the day of those records. A note bilbo holds no source and no time for has an empty first line. A note kept with no file has no source: its first line is the day it was kept. `bilbo read 01M6ZGE4KTBYYH07AS2KVMSCAF#g1:r3-19` prints them. A note too long for one chunk is printed in parts, each under that line. A note with so many sources that the line leaves no room in a chunk is printed without it: bilbo read of the hit's ref prints the note whole.

### Copies and long records

Three rules keep an answer from holding the same text twice. They apply in rank order, to the text a hit is printed with, the cut last hit included. The rules run over both lists, the chunks first: a note that repeats a hit above it is passed over, and a chunk is never passed over for a note. On a later page ([Pages](#pages)) what the earlier pages showed counts as above: their chunks for a chunk, their chunks and their notes for a note.

- A hit whose chunk equals the chunk of a hit above it is passed over.
- A hit is passed over when half or more of its runs of 5 words in a row are in the hits above it (`recall.copy_share` and `recall.copy_words`; a `copy_share` of 0 turns the copy rules off). A hit that is a copy as a whole but does not fit the budget is judged by the part that would be printed, and is printed when that part is new.
- No record is in more than `recall.record_cap` hits of its entry (2 by default; 0 sets no limit): the pieces of one long tool output are shown twice, not four times.

A passed-over hit uses no budget, and the next one is tried. stderr says how many were passed over: `passed over: 3 copies, 1 further part of a record`.

### The budget

`recall.budget` (4,000 tokens by default; `--budget <tokens>` sets it for one run, 256 at least) bounds the chunks of the plain output as the index's tokenizer counts it, each hit's header included. `recall.notes_budget` (2,500 tokens by default; 0 prints no note; it has no flag) bounds the notes the same way. When the search has no note to rank (none matches the query, or `--kind` or `--in` leaves the notes out), the chunks also take the notes' room. On a terminal a control character, and each of the twelve bidirectional controls (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069), prints as its escape, which the budget does not count, so the output can be longer there. In each list whole hits are printed while they fit. The first that does not is cut at a token and marked `cut`, and its list ends there for that page; a cut hit that is passed over as a copy does not end it. A note is never cut inside its source line, when it has one: one whose source line does not fit is left out. A note whose source line leaves its part no room, a hand-written one of about 25 sources or more at the defaults, has its paragraph on line 1. A hit that has room for no token of text is left out.

### Pages

`--page <n>` prints page `n` of the answer, a whole number from 1 to 18446744073709551615; without it, page 1. A page is what `n` runs in a row would print with one memory of what was shown: each page packs the chunks into their room and the notes into theirs, each list going on where its page before stopped, and the copy rules and the record cap count every earlier page ([Copies and long records](#copies-and-long-records)). A hit that was cut counts as shown: the next page goes on after it, and `bilbo read <ref>` prints it whole. A hit that had no room at the end of a page leads the next one. A list ends, for every page, at a hit that a page's whole room cannot show, as page 1's list does: a note whose source line alone is longer than the notes' room. With `recall.notes_budget` at 0 the notes have no pages. A run stops walking when both lists have ended, so a page far past the end costs what the page after the last costs.

A page that shows a hit, and after which a list has not ended, ends with one more line: `more: ` and the command that prints the next page, with the `--in` values, the kinds and the `--budget` the run was given.

```text
more: bilbo recall --page 2 --in=garden --budget 1500 -- 'thin the carrot rows'
```

Each value is written for a shell. A query or an `--in` value that holds a control character gives `more: the same command with --page <n>` instead. The line counts in neither room. It promises candidates, not hits: when all that is left is passed over, the next page prints nothing, says `nothing on page <n>: the list ended on page <m>` and exits 1. Between two pages the index can change; a page is made from the index as its own run finds it. `passed over:` and `left out` count the pages before it too. A program that reads the answer makes the next page's command from its own arguments; it does not run a line it read.

### Keeping the search to some entries

With no `--in` everything is searched: there is no project filter and nothing is read from where the command runs. `--in <value>` keeps the search to the entries its value names; the filter is applied inside the ranker, before the candidates are cut, so a small collection still fills its list. A value is one of:

- an entry id, whole or as a hit prints it (its last 10 characters, in upper case), with or without `bilbo:` and a locator, which is ignored;
- a tag, written `tag:<name>` as `bilbo tag` and `bilbo read` print it: every entry that carries exactly that tag. `--in tag:carpentry` does not name an entry for being tagged `carpentry-docs`, nor a session for having worked in a folder named `carpentry`: only the tag `carpentry` does, on the entry or on one it belongs to. What follows `tag:` must be a tag's name, or the run ends with a usage error, exit 2;
- words of a label: every entry with a label that holds those words next to each other and in order. Words are folded as a query's are (accents are dropped), cut at anything that is not a letter or a digit, lowercased and stemmed, so `--in reuniao` and `--in reunião` both name an entry labelled `/home/me/reunião`, `--in bilbo` names an entry labelled `/home/me/src/bilbos`, and `--in home-me-src-shed` names one labelled `/home/me/src/shed`. A session is in the scope of every folder it has as a label, so `--in /home/me/src/shed` keeps the sessions that worked there or were given it. A label is found by its first 64 words, and a value of more than 64 words names no entry. `bilbo read <id>`, a ref without `#r`, lists an entry's labels. A tag is a label too: `--in rust` names the entries tagged `rust` or `rust-async` with the entries whose other labels hold the word.

A value with a lower-case letter is never an id, so a word can always be given as one. A value that looks like an id (such as the word `REFERENCES`, which has the letters of a short id) and that no entry's id ends with is read as words. A value of ten digits that an entry's id ends with is that id. A short id that several entries end with names none of them: stderr lists them, and the value is not read as words.

A value is read as a tag first, then as an id, then as words. A tag's name can be 26 digits, which can also be an entry's id: `--in tag:<digits>` names the tag, and the bare digits name the entry with that id when the store holds one, and are read as words otherwise.

An entry that belongs to a kept entry through an id label comes with it, however deep: `--in <a session's id>` keeps the session and its subagents. Several values add up; `--kind` narrows the result. A project's Markdown files are not in its scope unless their folder is named too: `--in bilbo --in notes`. An entry's notes come with it.

A value that names nothing is said on stderr, `--in '<value>' names no entry`, and the search goes on with the other values; when no value names an entry nothing is searched, `nothing matched` and exit 1: `--in` never falls back to searching everything. A value with no letter or digit, such as `.`, is a usage error, exit 2, before the store is opened.

A value that starts with a dash is read by the command line as an option: give it as `--in=-home-me-src-shed`, the form of the folder name Claude Code gives a project. After `--in` and a space such a value is a usage error, exit 2: `--in -home-me-src-shed` says `--in needs a value`.

stderr says what the scope kept and what it cost: `--in kept 1,203 entries and left out 31 of the best 400: markdown 7, session 24` counts the entries kept, then how many of the 400 best matches without the scope fall outside it, by kind.

Labels are filed when the index is made, by `bilbo remember`, `bilbo refresh`, `bilbo forget` and `bilbo reindex`. An index that cannot be compared with the store's catalog (the catalog was made again) holds no label until the next run, so a value read as words can name fewer entries than it should: stderr says so, after the line about the index, and ids still work. An index behind the store knows the labels of what it indexed. Ids and tags are the store's own and work at once.

### What is searched

The index is searched as of its last commit. stderr says how many entries it holds: `searched 1,412 entries` counts the entries of the index, not of the store, and is not printed when a scope named nothing.

### When the index is behind

Three lines on stderr say what stands between the index and the store. An index behind the store says by how many entries (`bilbo refresh` brings it up to date); an index that cannot be compared with the store's catalog says each hit was checked against the store; hits the store no longer gives as indexed (a Markdown file that grew in place, an entry rewritten or gone) are left out and counted. A hit is shown only when the store still gives the text the index ranked: its chunk is recomputed from the entry's records and compared with a digest the index holds. A session that only grew still gives its indexed chunks. The first search after an upgrade that adds a kind says the index was made by another bilbo, prints no hit and exits 1, until `bilbo refresh` makes the index again.

### The query log

Each query that reaches the store is logged in `queries.log` in the state folder ([Configuration](configuration.md#folders)): whatever it finds, also an index that was refused or damaged. A run that ends before the store is open (a usage or config error, a tokenizer that does not load, no store, a store held past `store.wait`) is not logged. One line a query, fields apart by tabs: `q4`, the time in unix seconds, `hits`, `nothing`, `no-index`, `other-manifest`, `withheld` or `failed`, the number of entries searched, the kinds asked (`-` for none), the `--in` values (`-` for none; a comma in a value is written `\,`), the number of entries they named (`-` for none, when the run ended `failed`, or when it ended `no-index`, `other-manifest` or `withheld` before it resolved them), the page the run was asked for (1 without `--page`), the printed hits of that page as `<id>#r<a>-<b>`, or `<id>#r<a>` for a ref of one record (a cut one followed by `!`), the words read as misspelt (`-` for none; each as `<term>><near>`, the terms searched in its place joined by `+`, the words joined by commas, as in `carot>carrot+cart,pyton>python`), and the query's text, with `\`, tab, line feed and other control characters escaped. The file holds your questions, stays in the state folder, is made with mode 600, grows without a bound and can be deleted at any time. A line that begins `q3` is an earlier bilbo's, with ten fields and no page: a reader goes by a line's first field. A line that cannot be written is one warning on stderr, `the query was not logged: <error>`; hits and the exit code do not change. A store with no state folder that bilbo checked is not logged and says `this store has no state folder that bilbo checked`. The line is appended under the state folder the store opened and checked at this run, and only a regular file of this user with one name and no group or other permission is written.

### Searching while bilbo writes

`recall` takes the store's lock in shared mode, as `read` does, and holds it until the last hit is read back. While a `bilbo remember` holds the store it waits up to `store.wait` (10 seconds by default), says so on stderr once, and then gives up with exit 1. It never takes the writer, and it creates and repairs nothing: no index, catalog or folder. It reads the index only when the cache folder is the store's own, private, and the state folder has a `store.lock`; otherwise it says so and exits 1.

### What stderr holds

Off a terminal every line bilbo writes starts with `bilbo: `. On a run that searched: `searched <n> entries`, `--in '<value>' names no entry` for each value that names none, `--in '<value>' ends the id of <n> entries: <id>, <id>; give one id in full` for a short id several entries end with, `--in kept <n> entries and left out <o> of the best <b>: <kind> <n>, ...`, `typos: <word> as <term>, <term>; <word> as <term>` (the words read as [misspelt](#misspelt-words), only when there are any), then the lines of the last section (and, when a value was read as words at an index that cannot be compared, `--in may name fewer entries than it should: labels are filed when the index is made; bilbo refresh brings both up to date`), `left out <n> hits ...`, `passed over: ...` (the copies, the further parts of a record and the hits over the entry cap that were passed over, each item only when it is over 0), `the query was not logged: <error>` and, with no hit, `nothing matched`, or `nothing on page <n>: the list ended on page <m>` for a page after the last. Where no search ran, an error line: `the store has no search index bilbo can use; bilbo reindex makes it`; `the search index was made with other settings or by another bilbo; bilbo reindex makes it again`; or the reason the index was not read: no `store.lock` in the state folder, a cache folder with no catalog of this store, with another store's catalog, or that is not this user's alone or could not be checked (`<path> is not this user's alone, or could not be checked; bilbo reads no search index under it`). A damaged index can make tantivy print a line of its own before bilbo's; bilbo's line is the last.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | at least one hit |
| 1 | nothing matched, a page after the last, there is no store, no search index or none that bilbo can read, the store is being written for longer than `store.wait`, or the store or the index could not be read |
| 2 | usage or config error: no query, an unknown kind, an `--in` value with no letter or digit, a `--budget` outside 256 to 1,000,000, a `--page` that is no whole number from 1 to 18446744073709551615, a store root that holds bilbo 0.20's `notes/` or `library/`, or a state or cache folder inside the root |

## digest

```text
bilbo digest [--agy]
```

Answers the first prompt of a session with pointers to what the store holds that bears on it, Markdown files, sessions and notes, each in a list of its own, and prints nothing for every later prompt of that session. A prompt hook runs it. It only reads the store: the two files it writes are in the state folder ([A session's first prompt](#a-sessions-first-prompt), [The digest log](#the-digest-log)). It always exits 0: a harness blocks a prompt whose hook exits 2, and no exit code of `bilbo digest` does that. How long a run can take is another matter ([While bilbo writes](#while-bilbo-writes)). The default and the range of every `digest.*` key are in [Configuration](configuration.md#keys).

### The hook's input

Standard input is one JSON object, as Claude Code and Codex hand one to a `UserPromptSubmit` hook. Two fields are read, `session_id` and `prompt`, each a string that is not empty. In either string an escape that is half of a surrogate pair with no partner, such as `\ud83d` where a harness cut a text inside an emoji, is read as replacement characters (U+FFFD). Every other field is passed over, whatever it holds and however deep, as long as it is JSON, a field whose name holds such a half among them; and of a field named twice the last is read. An input of up to `digest.input` bytes is read whole; a longer one is no prompt, and the run stops reading one byte past the limit. An input that is not a JSON object or that lacks one of the two fields is no prompt either: the run prints nothing, marks nothing and says why on stderr. With a person at the terminal it reads nothing and says what it reads.

With `--agy` the input is the one AGY hands a `PreInvocation` hook, before every model call. Three fields are read by the same rules: `conversationId`, which is the session, `transcriptPath`, and `invocationNum`, 0 when it is absent. A call whose `invocationNum` is not 0 starts no turn: the run ends at once, prints and says nothing, and is not logged. Otherwise the prompt is the last user input of the transcript: the last `digest.agy.tail` bytes of that file are read as lines from the end; a step whose `type` is `SYSTEM_MESSAGE` is passed over, and the first step whose `type` is `USER_INPUT` gives its `content`, or what stands between `<USER_REQUEST>` and `</USER_REQUEST>` in it. Any other step first, a line that is not JSON, or no text left is no prompt: `the transcript's last steps hold no user input`. Only a regular file is read, and no more than that many bytes of it: a transcript that is a folder, a pipe or a device is no prompt, `the transcript <path> is not a regular file`.

### A session's first prompt

A session is marked when its first prompt is seen, before anything is searched: one line in `digest.sessions` in the state folder, with the time and the session's id. A prompt whose session has a line there prints nothing, and opens neither the store nor the index. The first prompt is the first one whatever it got: a prompt nothing matched, one whose every match was under its list's floor and one that found the store held each use the session's one answer. A run that is stopped between the mark and its answer leaves a session with no answer; one that is stopped later may have printed it. While the state folder and the file stay as they are, no session is answered twice, also when two prompts of one session come at once. When the mark cannot be read or written the run prints nothing and says so, at every prompt, until it can: a mark that was not written is no mark. The file is made with mode 600, grows by one line a session without a bound and can be deleted at any time: a session whose line is gone, with the file or with a part of it, is treated as new at its next prompt, once: that prompt gets what any first prompt gets, which can be nothing. A line that was cut off as it was written marks nothing, and is removed when the next session is marked.

### The prompt as a query

The prompt is the query, cut to its first `digest.query` characters: a search takes longer the more words it has and the more the store keeps, sessions included, and the hook holds the prompt meanwhile. It is cut into words and ranked as [`recall`](#the-query) ranks a query, with `recall`'s keys: `recall.question_words`, `recall.languages`, `recall.bm25_k1`, `recall.bm25_b`, `recall.read_batch`, `recall.pair_weight` and the `recall.typo_*` keys. A slash command is searched by its words.

### The lists

Every kind bilbo reads has a list of its own: the Markdown files, the sessions, and the notes, which are the paragraphs `bilbo derive` wrote and the texts kept with [`bilbo remember -`](#a-note-with-no-file). No key names the kinds: a list is on unless its count is set to 0. A list's chunks are ranked among themselves: every number of a score, and the terms searched for a misspelt word, are counted over the chunks of its kind alone. So a Markdown file scores what it would score in a store that held nothing else, however many sessions the store keeps. No score of one list is compared with a score of another: a word that is rare among the files can be common among the sessions, and each list has its own floor. Of a list's best `digest.candidates` chunks each entry is taken once, at the score of its best chunk that is not passed over ([What a list leaves out](#what-a-list-leaves-out)), the best first, and at most `digest.<kind>.most` entries are taken. Of the entries taken, those at their list's floor or over are chosen for the list ([The floor](#the-floor)), and a row is then made of each chosen entry. An entry is what the store keeps as one: a session's file is one entry and has one row at most, however many of its passages match, and one session's file kept from two folders is two entries. The notes `bilbo derive` wrote from one session are one entry too, so the notes' list shows one note of a session at most, its best. None of `recall`'s budgets, caps and copy rules apply. The lists are printed in the order files, sessions, notes, and one with nothing to show is left out.

### What a list leaves out

- **Every list:** an entry the store no longer holds as the index has it, such as a file withdrawn since the index was made; for a row that names records, a passage the store no longer holds as indexed, such as one of a session's file that was written anew; and an entry whose records the store cannot read.
- **Every list:** the session the prompt came from, with what belongs to it (its subagents' files, the notes written from it), when the hook's `session_id` is the whole UUID that session's file is named by, in either case of its letters. A prompt needs no pointer to its own session.
- **The sessions' list, while `digest.session.subagents` is `false`:** a subagent's file. It is a part of the session it worked for, and that session can be listed.
- **The sessions' list, while `digest.session.parts` is `false`:** a part of a record too long for one chunk, such as a long tool output. The session can still be listed at another passage.
- **The sessions' list:** a session that began longer ago than `digest.session.old_after`. It goes by the one date the index holds for the session: the time of its first passage that has one, or the time bilbo captured the session when no passage has a time. It does not go by when the passage was written: a session that began before the limit is left out whole, and the day a row shows is its passage's own.
- **The notes' list:** a note written from an entry that an earlier list of the same run chose. A session and the notes `bilbo derive` wrote from it are never both listed: once the session is chosen for its list, its notes are hidden from the notes' list, and the session's row carries the start of one of them, as its fifth field, where one was written from records of its passage and can be read. The notes of a session that was not chosen are not hidden. They are ranked as every note is, and one of them is a row only when it earns a place in the notes' list by its own score. What this rule keeps from an agent has not been measured.

The two switches are the sessions' list's alone: the files' and the notes' lists leave out neither, since a note `bilbo derive` wrote belongs to the entry it was written from.

What belongs to a session is what bilbo keeps as belonging to it. A subagent's file kept before its session, and not grown since, carries no mark of that session: it is an entry of its own and can be listed, also for a prompt of the session it worked for.

Three things are told apart: an entry chosen for a list, an entry a chosen one hides, and a row that was made. What is left out before a list is chosen takes no place: the list goes on to its next best chunk, of the same entry or of another, until it has its count. Every list is chosen before any row is made. An entry whose records cannot be read is found only when its row is made: it has no row, nothing takes its place, and what it hid stays hidden for that run. So a chosen session whose records cannot be read is not listed, and neither are the notes written from it.

### The floor

An entry taken for a list is chosen when its score is at the list's floor or over; one under the floor is held back, and hides nothing. A floor of 0 chooses every entry taken. When no list has an entry at its floor or over, the run prints nothing. A score grows with the words a prompt shares with an entry, so a longer prompt passes a floor more easily. It also changes with how much of a kind the store keeps and with how common the prompt's words are there, so a floor is a number for one list, and for a store of about one size.

| Key | Where its default comes from |
|---|---|
| `digest.markdown.floor` | Measured: a month of first prompts over some hundreds of Markdown files |
| `digest.session.floor` | Read off one set of labelled sessions, and not confirmed on another |
| `digest.note.floor` | Not measured: the number was taken from the one measured for Markdown files. A short note scores high |

The last two are starting points. [The digest log](#the-digest-log) holds what each list showed and what its floor held back, with the scores: set them by it.

### Output

Standard output is empty, or for each list that has a row one line and then its rows:

```text
files: 2 may bear on this prompt, best first; bilbo read <ref> outlines one
2T6W9YB0DG<TAB>file:/home/me/notes/garden-log.md<TAB>The garden log<TAB>summary: The vegetable beds since March: what was sown in each, the thinning dates of the carrots, and the drainage of the north bed, which is still open.
5N3K8QW1ZX<TAB>file:/home/me/notes/shed-roof.md<TAB>The shed roof
sessions: 2 may bear on this prompt, best first; bilbo read <ref> prints one
AS2KVMSCAF#r3-7<TAB>2026-11-07<TAB>/home/me/src/garden<TAB>user: Thin the carrot rows, then tell me how many seedlings are left.<TAB>note: The user asked the assistant to thin the carrot rows in the north bed of /home/me/src/garden and to count the seedlings that were left. The assistant counted 142 seedlings in four rows and told the user to wait for the second leaves before thinning again, because the first leaves tear when the soil…<TAB>summary: Thinning the carrots of the north bed: 142 seedlings counted in four rows, and a second thinning planned once the second leaves show.
Q4W7N9R3HB#r18-21<TAB>2026-11-02<TAB>/home/me/src/garden<TAB>user: Here is the bed. What is the weed in the corner?
notes: 1 may bear on this prompt; bilbo read <ref> prints it
M4P6R8T0V2#r1<TAB>2026-11-06<TAB>-<TAB>The beet seeds are in the drawer by the door.
```

A list of one row says so: `files: 1 may bear on this prompt; bilbo read <ref> outlines it`. Fields are apart by tabs.

- **A file's row** is its ref, its capture label and its title, which is the file's first heading. The ref names the file and no record: `bilbo read <ref>` prints the file's labels and its [outline](#the-outline), and each line of the outline starts with the locator that reads that section.
- **A session's row** is the ref of a passage, the passage's day in UTC, the first folder the session's records name, and the last line the session's [outline](#the-outline) has at or before the passage: most often a prompt a person typed, sometimes a compact summary's line or an agent's message. A passage is a range of records and can hold more than one turn: the row does not say which of its records held the prompt's words. The ref names records of the session as it is kept when the digest runs. `bilbo read <ref>` prints that range of the session as it is kept when the command is run, a page at a time.
- **A note's row** is the note's ref, the day of the records it was written from or the day it was kept, `-`, and the note from its start. `bilbo read <ref>` prints the note under its first line: what it was written from, or the day it was kept.
- **A fifth field,** `note: ` and a text, is the start of a note `bilbo derive` wrote from records of that passage. It is a model's words about those records, as they were when it ran; the ref still reads the records themselves. Of several such notes the row shows the one that shares the most records with the passage. A session `bilbo derive` wrote nothing about has none. Whether this text helps an agent choose what to read has not been measured.
- **A last field,** `summary: ` and a text, is the summary `bilbo derive` wrote of the row's whole entry, a file, a session or a note kept with [`bilbo remember -`](#a-note-with-no-file), where the store holds one ([Summaries](#summaries)). It stands after every other field, a `note: ` field among them. It is a model's words about the entry as it was when this summary was written. It is printed whole, with each run of white space as one space, as `bilbo read` of the entry prints it: no key cuts it, and what keeps it short is `derive.summary.most`, the length the call asks for, which an answer may pass. With `digest.summary.enabled = false` no row has one and none is looked up; `bilbo read` prints it all the same. An entry `derive` wrote no summary of has none: a note `derive` wrote is such an entry, since it belongs to the session it was written from. Whether this text helps an agent choose what to read has not been measured.
- **What a row shows of a note,** in its line or in a fifth field, is the note from its start, with each run of white space, line feeds among them, as one space, cut to `digest.note.width` characters. For a row of the notes' list, `bilbo read <ref>` opens the note itself, a page at a time. A fifth field has no ref of its own, since the row's ref reads the session's records: [`bilbo recall`](#notes) is the way to that note's ref, which `bilbo read` then opens.

Every ref is what `bilbo read` takes: the last 10 characters of the entry's id, or the whole id when another entry ends alike, and for a passage its records as `#r3-7` or `#r4`. A field with nothing to say is `-`. A title and an outline's line longer than `read.outline_width` characters are cut, and so is a note longer than `digest.note.width`; what is cut ends in `…`. A tab or a line feed in a field is written `\t` or `\n`, as `bilbo tag` writes it.

A harness adds these lines to what the agent reads with the prompt, up to a size: Claude Code caps what a hook prints at 10,000 characters, and Codex at about 2,500 tokens. Over the cap a harness writes the output to a file and hands the agent that file's path in place of the lists; Claude Code adds their first 2,000 characters. Three lists of six rows each, every session's row with a note and every note cut at 300 characters, are about 5,800 characters for rows of ordinary size. A summary is printed whole, and what `derive.summary.most` asks for is no bound: asked for its default of 200 characters, a model wrote about 280 in the four answers this estimate was read off. With a summary of that length on each file's and each session's row they are about 9,300, which is under Claude Code's cap and near Codex's. That is an estimate and no bound: a file's capture label and a row's folder are printed whole, so long ones, or text that is dense in tokens, can reach a cap at those settings too. Counts or a width far over those, or summaries longer than that, can pass it.

With `--agy` the lists are printed as one line, the JSON object AGY adds to the conversation as a system message: `{"injectSteps":[{"systemMessage":{"systemMessage":"<the lists>"}}]}`, the lists as above with a line feed between their lines.

### The digest log

Each run that reaches the state folder is logged in `digest.log` there, unless `digest.log.enabled` is `false`. A run that ends before (a usage or config error, no store, a state folder bilbo did not check) is not logged. One line a run, fields apart by tabs.

Six fields come first: `d1`; the time in unix seconds; `listed`, `nothing`, `floor`, `later`, `input`, `unmarked`, `busy`, `no-index`, `other-manifest`, `withheld` or `failed`; the session's id, or `-` when the input gave none (an id that is itself `-` is written `\-`); the milliseconds from the verb's start to the moment the line was made, which leaves out the start of the process, the writing of this line and the printing of the lists; and `current`, `behind` or `unknown` for the index in a line that says `listed`, `nothing` or `floor`, or `-` in any other line.

Then, in a line that says `listed`, `nothing` or `floor`, five fields for each list that is on, in the lists' order: the list's kind (`markdown`, `session`, `note`); its floor; what it listed, the best first, joined by commas; what its floor held back, in the same form; and what it left out on the way, each cause as `<cause>=<count>`, joined by commas. A listed file is `<id>=<score>`, with `*` after the score when its row carried a summary; a listed passage is `<id>#r<a>-<b>=<score>`, or `<id>#r<a>=<score>` when it is one record, with `+` after the score when its row carried a note's text, and `*` after that when it carried a summary. The causes are `gone` (the store no longer holds it, or could not read it), `own` (the prompt's own session), `belongs` (a subagent's file, while `digest.session.subagents` is `false`), `part` (parts of long records, while `digest.session.parts` is `false`), `old` (began longer ago than `digest.session.old_after`) and `shown` (written from an entry an earlier list chose). An entry is counted once, under the first of `gone`, `own`, `belongs`, `old` and `shown` that holds for it; `part`, and `gone` for a passage the store no longer holds, count passages. They count what was met before the list was full, and one thing after it: a chosen entry whose records then could not be read is in no row, is not among what the list listed, and adds one to `gone`. Each of the last three fields is `-` when it is empty. A list that is turned off has no fields. Every other line has the first six alone, a `failed` one too, whatever the run had ranked before it failed.

`listed` says that some list has a row, `floor` that none has and some floor held an entry back, `nothing` that neither is so. `later` is a prompt of a session that was marked before. `listed`, `nothing`, `floor`, `busy` (a store held for longer than `digest.wait`), `no-index`, `other-manifest`, `withheld` and `failed` are a session's first prompt. `input` is an input that was no prompt and `unmarked` a prompt whose session could not be marked: neither says which prompt of its session it was.

The file holds no text of a prompt and no text of an entry, so no title, folder or line of a row: the session's id stands for the prompt, and `bilbo read` finds the session by that id when it is the UUID its harness gave it ([The ref](#the-ref)), once the session is kept. A hook may hand any text as an id: of one that is no such UUID nothing is promised. The file is made with mode 600, grows without a bound and can be deleted at any time. A line that was cut off as it was written is removed before the next line is written, so each run's line stands whole and alone. A line that cannot be written is a warning on stderr, `the digest was not logged: <error>`; the rows do not change. A harness that stops a run can leave a mark with no line, or a `listed` line for lists no agent saw.

### While bilbo writes

A later prompt waits for no writer of the store. A session's first prompt reads the store as `recall` does, and waits for a bilbo that is writing it up to `digest.wait`, not `store.wait`; then it gives up, prints nothing and says `another bilbo is writing the store`. The session is marked all the same. `digest.wait` bounds that wait alone. It keeps the store open for reading until its rows are made, as `recall` does until its hits are read. A prompt of a session also takes the lock of `digest.sessions` once it has opened that file, which an input that is no prompt never does, nor a run with no state folder or with a marks' file it refuses; and every run that opens `digest.log` takes that file's lock: another `bilbo digest` holds each for one read and one write, and no key bounds those waits. When the catalog is missing or behind, a first prompt walks every segment as any reader does, which takes seconds on a large store (about three for a store of one gigabyte); with no catalog at all it then lists nothing, as `recall` would. Keep the hook's timeout over that, and `bilbo refresh` on its timer. A harness that stops a run at its timeout can do so at any point: before the session is marked, after the mark and before the lists, or once the lists are out.

### What stderr holds

Nothing, for a run that listed, found nothing, held every entry back or was a later prompt. Nothing either for an entry whose records could not be read: it is left out, as `recall` leaves such a hit out ([What a list leaves out](#what-a-list-leaves-out)). And nothing for a session's notes that could not be read: its row comes without its `note: ` field. Nor for a summary that could not be read: its row comes without its `summary: ` field. The two are looked up apart, so a row can carry one with the other missing. Otherwise: a usage or config error as every verb prints it; why the input was no prompt (`cannot read standard input: <error>`, `the hook's input is longer than digest.input allows (<n> bytes)`, `the hook's input is not a JSON object`, `the hook's input has no session_id`, `the hook's input has no prompt`, and with `--agy` `the hook's input has no conversationId`, `the hook's input has no transcriptPath`, `the transcript <path> is not a regular file`, `cannot read the transcript <path>: <error>`, `the transcript's last steps hold no user input`); `this store has no state folder that bilbo checked`; `the session was not marked, so nothing was listed: <error>`; or the store's or the index's error, in `recall`'s words. A run says one of these at most. After it, or alone, it says `the digest was not logged: <error>` when the log's line could not be written: a full disk can give both.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | always: the lists, nothing to list, a later prompt, the page of `bilbo digest --help`, and every error, a usage or config error among them, also when the result cannot be written, of which a part may then have gone out |

The code says nothing of how long a run took. `bilbo help digest` is `help`'s and exits as it does for every verb.

## read

```text
bilbo read <ref> [--budget <tokens>]
bilbo read <ref> --outline [--budget <tokens>]
```

Prints an entry bilbo keeps: its metadata and its outline, or its records a page at a time. Each page ends with the command that prints the next one, so a reader that follows those lines sees every record once.

### The ref

```text
<ref>     = <entry> | <entry>#<locator>
<entry>   = <id> | bilbo:<id> | <short id> | <session id>
<locator> = r<a> | r<a>-<b> | g<n> | g<n>:r<a> | g<n>:r<a>-<b>
```

- `<id>` is the entry's id, 26 characters, with or without the `bilbo:` a citation gives it.
- `<short id>` is the last 10 characters of an entry's id, in upper case, as `bilbo recall` prints it: `bilbo read 2T6W9YB0DE#r3-7`. When several entries end with it, the run exits 2 and lists them: give the id of one.
- `<session id>` is the UUID a harness gave a session, or its first 8 characters or more: `bilbo read 2d6b1857`. It names the entry kept from the `.jsonl` file whose name ends in that UUID: Claude Code's `<uuid>.jsonl`, Codex's `rollout-<time>-<uuid>.jsonl`. A Markdown file named for a UUID is not a session: read it by its id. Upper or lower case. When it names several entries (one session kept from two folders, or a short prefix two sessions share), the run exits 2 and lists up to `read.matches_listed` of them (five by default) with their capture labels: give the id of one. A subagent has no UUID of its own: read it by its id, which its session's records name. A forgotten session is not named by its session id either: read it by its id.
- `r<a>-<b>` is the records from `a` to `b`, counted from 1; `r<a>` is one record. A record outside the entry is refused, exit 1, with the number of records the entry has.
- `g<n>` is a generation: an entry whose file was rewritten keeps the old records as an older generation. With no `g`, the newest. A page of an older generation shows it in every locator (`#g1:r5`); a page of the newest shows the short form (`#r5`). The `more:` and `cut:` commands of the last line always name the generation that was read, so a file rewritten between two pages cannot make the next page read other records.

### The outline

With no record range, `read` prints what it knows of the entry, then one line per prompt of a session or per heading section of a Markdown file, each after the locator that reads it:

```text
bilbo:01M6ZGE4KTBYYH07AS2KVMSCAF
kind: session
label: file:/home/me/.claude/projects/-home-me-src-garden/8f2b6d4e-1a3c-4e5f-9b7a-2c4d6e8f0a1b.jsonl (capture)
label: bilbo:01M6ZFZ0V5T2K1X8Q4W7N9R3HB
label: tag:garden
label: /home/me/src/garden
removed: tag:shed
captured: 2026-11-09 03:12 UTC
started: 2026-11-07 08:01 UTC
records: 23
generation: 1
outline:
#r3 user: Thin the carrot rows, then tell me how many seedlings are left.
#r17 compact summary: This session is being continued. The carrots in the north bed were thinned from 42…
#r18 user: Here is the bed. What is the weed in the corner?
end: #r1-23 of 23 records
```

- A `label:` row is one label of the entry. The capture label, the one `remember` finds the file again by, comes first and is marked `(capture)`. A note kept with no file has the capture label `note:<id>` until it is forgotten, which names no file and is no ref. A forgotten entry's capture label begins `forgotten:` ([The label](#the-label)). `bilbo:<id>` is the entry this one belongs to (a subagent names its session); `tag:<name>` is a tag the entry carries, in name order, and `(model)` marks one that `bilbo derive` declared from a model's answer; the rest are the texts the entry's records name, in byte order, as many as fit in half the page's budget, and then one row, `labels: <n> more; a larger --budget lists them`. A session names the folders it worked in and the folders it was given. A Claude Code session names the `cwd` of its records, the added directories of its environment records, and the folders of the project instructions files that its instructions records list. A file in a `.claude` folder, or under the `rules` folder of one, counts as a file of the folder that holds that `.claude`, the nearest to the file when a path has several. Such a record lists every file Claude Code loaded on its way up from the record's `cwd`, so a folder above that `cwd` is not taken from it, and such a record with no `cwd` to compare with gives none. The instructions file Claude Code lists as the user's own and the memory file give no folder. A file that a project instructions file imports is listed as any other, so an import from another folder can make that folder a label. A Codex rollout names the `cwd` of its `session_meta` and `turn_context` records and the `runtime_workspace_roots` of its `session_meta` records. A text the store replaced by an opaque placeholder (a very long folder with no dot or space in it) is not a label. An entry with no label has no `label:` row. `started` is left out for an entry whose records carry no time (a Markdown file).
- A `summary:` row, the last before `outline:`, is the summary `bilbo derive` wrote of the entry ([Summaries](#summaries)), whole, with each run of white space as one space. It stands on a read of the entry's newest generation alone: an entry with none has no such row, a page of records never has one, and neither has `#g1` of an entry rewritten since. The row counts in `--budget` as the rest of the header does.
- A `removed:` row is a name you took off the entry with `bilbo tag --remove`: `bilbo derive` will not declare it for this entry again. The tags and the removed names come from the store and are always listed, whatever the budget. A read of a record range prints no `label:` and no `removed:` row.
- `generation` is `1 of 2` when the one shown is not the newest.
- A session lists its user prompts, its compact summaries and, for Codex, the messages one agent sent another. A note entry lists the first line of each note's paragraph that is not blank; a Markdown file lists its frontmatter, the text before its first heading, and each heading with the headings above it: `#r28-31 The garden log > Watering > Tools`.
- An outline line is the prompt's first line, at most `read.outline_width` characters (100 by default); a longer one is cut to fit and ends in `…`. Inside a Markdown file's path, a heading over `markdown.heading_width` characters (100 by default) is cut to fit and ends in `…` too.
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
- A Markdown file's records are its lines. They are printed a section at a time under the locator of the lines printed: `#r28-31`. A range that starts or ends inside a section prints the lines it holds. Headings are found by scanning lines, not by parsing Markdown's containers: inside or right after a list, a quote or indented code a section can start at the wrong line or carry the wrong label, but every line is printed, once.
- A note entry's records are one note each. A page prints each note under `#r<n>`, its first line the note's source line, empty when it has none; a record that holds no note prints nothing.
- A pipe or a file gets each record's text as it is. A terminal gets its control characters and the twelve bidirectional controls (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069) escaped (`\u{1b}`), also when an agent runs bilbo there; a person's terminal also gets the locators in bold and the commands in colour.

The last line of every output is bilbo's, on a line of its own also when the last record has no line end, and nothing else is: a record prints as it was kept, so a line above the last can look like a locator or like any line of bilbo's. It is one of two:

| Line | Meaning |
|---|---|
| `more: bilbo read <id>#g<n>:r<a>-<b>` | The command for the next page: the rest of what was asked, from the next record that renders, in the generation this page read. It carries `--outline` after an outline page and `--budget <n>` when the flag gave the budget. |
| `end: #r<a>-<b> of <n> records` | Nothing is left of the range this run was asked for; the entry has `n` records. An entry with no record ends with `end: 0 records`. |

When the page ends in a record that was cut, the same line says so first: `cut: <n> more characters; bilbo read <id>#g<n>:r<k> --budget <b>; more: ...`, or `...; end: ...` when nothing is left after that record.

### The budget

A page holds as many whole records as fit `read.budget` tokens (see [Configuration](configuration.md#keys)); `--budget <tokens>` sets it for one run. A token is counted as 4 bytes until bilbo's tokenizer arrives, so the default of 7,000 is at most 28,000 bytes of records: under the 30,000 characters at which Claude Code cuts a command's output. That bound holds for a pipe or a file, which is what a harness reads. On a terminal each control character of a record, and each of the twelve bidirectional controls (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069), prints as its escape, which the budget does not count, so a page can be longer there.

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

The cut falls between two characters. The budget of the `cut:` command is what the whole record counts under the locator of that command, rounded up to the next thousand; it is always one `--budget` accepts, and it prints the record whole also after the file was rewritten. The cut record counts as printed: `more:` goes on after it. A long section of a Markdown file is not cut: it goes on at its next line on the next page. Only a single line over the budget is cut.

### Reading while bilbo writes

`read` only reads: it creates and changes nothing in the store, the state or the cache folder. It holds the store's lock in shared mode for as long as it takes to fetch the entry's records, and lets it go before it renders or prints. While a `bilbo remember` holds the store, `read` waits up to `store.wait` (10 seconds by default), says so on stderr once, and then gives up with exit 1: `another bilbo is writing the store`. Run it again.

`read` uses the catalog in the cache folder when it holds every frame of the store. When the catalog is missing or behind (the cache folder was deleted, say), `read` walks every segment instead, under the lock, which takes as long as the store is large: reads are slow until the next `bilbo refresh` rebuilds the catalog. A `bilbo remember` or a `bilbo refresh` that finds a reader on the store waits for it.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when no record of the range renders |
| 1 | no entry has the id or the session id, the range or the generation is outside the entry, the entry is of a kind this bilbo cannot read, there is no store, the store is being written for longer than `store.wait`, or the store could not be read |
| 2 | usage or config error: a ref that does not parse, a session id or a short id that names several entries, a `--budget` under 256 |

## tag

```text
bilbo tag
bilbo tag tag:<name>
bilbo tag <ref>... [--add <tag>]... [--remove <tag>]...
```

Lists the tags, lists the entries that carry one, or changes the tags of entries.

A tag is a label somebody declared for an entry: a person, an agent that passes it, or `bilbo derive` from a model's answer when you turn that on. bilbo reads no tag from what an entry holds. There is no list of allowed tags: a tag exists while an entry carries it.

### The tags

`bilbo tag` prints one row a tag, in name order, its fields apart by tabs: the tag as `--in` takes it, how many entries carry it, and how many of each kind.

```text
tag:carpentry-docs	40 entries	markdown 40
tag:garden	213 entries	markdown 4, session 209
tag:rust	117 entries	markdown 117
```

With no tag in the store it prints nothing, says `no entry carries a tag` on stderr and exits 0.

### The entries of a tag

`bilbo tag tag:<name>` prints one row an entry that carries the tag, in id order, its fields apart by tabs: its id, its kind, its capture label, or `-` for an entry with none, and who declared the tag for it, `person` or `model`. A tab or a line feed in a capture label is written `\t` or `\n`, as `bilbo read` writes it, so a row is always four fields on one line. `bilbo read <id>` reads one. A tag no entry carries is `tag:<name> names no entry`, exit 1.

### Changing tags

A `<ref>` is what `read` takes for an entry, with no `#` part: a tag is on an entry, not on records. Every ref is resolved, and every entry checked, before anything is written; an entry named by two refs is one entry. A ref gets the answer `read` gives it, in `read`'s words, and nothing is written: one that names no entry ends the run with exit 1; one that names several entries, or that does not parse, is a usage error, exit 2. Each entry then carries the tags it had, with those of `--add` and without those of `--remove`. Adding a tag an entry does not carry yet, when that would leave it with more than `tag.cap` tags, 32 unless set, ends the run with exit 1 and nothing written. A tag the entry already carries can always be made yours, whatever the number of its tags.

`--add` makes a tag yours, whoever declared it. `--remove` takes a tag off an entry and keeps its name with the entry: `bilbo derive` never declares a name you removed from an entry, until you declare it there again, with `--add` or with `bilbo remember --tag`. Removing a tag the entry does not carry changes nothing. `bilbo read` shows both, the tags and the removed names.

Standard output, one line: `changed <n>, unchanged <n>`. An entry is changed when anything of its tags was written, so an `--add` that makes a model's tag yours counts. A tag no entry carried before the run is said on stderr, `tag:<name> is new`: a typo makes a tag. Changing tags writes the store, so it waits for another bilbo that writes it, as `remember` does, and it closes as `remember` does: it serves a request another bilbo left for one more pass over the follows ([Two runs at once](#two-runs-at-once)), with `remember`'s lines on stderr, and seals when a seal is due. The change itself makes no index again: `recall --in tag:<name>` sees it at once.

`derive.in` and `derive.not_in` are read when a `derive` run plans. A tag you add or remove while a run is under way counts from the next run: to keep an entry from a provider, tag it before the run that would send it.

The store only grows: a tag that was removed is no longer the entry's, and the frame that held it stays in the store.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when nothing changed and when the store has no tag |
| 1 | a ref that names no entry, a tag no entry carries, a new tag that would leave an entry with more than `tag.cap` tags, a store failure, or a pass over the follows run again for another bilbo failed or refused a file |
| 2 | usage or config error: refs with neither `--add` nor `--remove`, either of them with no ref, a name that is no tag, a tag both added and removed, a ref with a `#` part, a ref that does not parse, a session id or a short id that names several entries, `tag:<name>` beside another argument |

## forget

```text
bilbo forget <ref>...
```

Takes entries out of `recall`. A `<ref>` is what `read` takes for one entry, with no `#` part: of the ref a `recall` hit prints, give what comes before the `#`. Every ref is resolved before anything is written.

Nothing is taken out of the store. An entry the run forgot keeps its id, its tags and every generation it had, and its newest generation holds no record: `recall` gives no hit of it, and `bilbo read <id>#g<n>` still prints any earlier generation it had, for a kind `read` reads. A run that finishes its writes then brings the search index up to date, as `remember` does, and says so when it cannot. A bilbo from before `forget` does not know a forgotten entry and may undo a forget.

### The label

A forgotten entry gives up its capture label, which another entry can then hold, for `forgotten:<id>:<bytes>:<digest>:<label>`: its id, the size and digest of what it held, and a capture label, at first the one it gave up. A session's id no longer names the forgotten entry; its own id still does.

### What belongs to it

What belongs to an entry is forgotten with it: the entries that belong to it through an id label, at any depth (a session's subagents), and the notes and the summary `bilbo derive` wrote of it and of each of those. Each can also be forgotten on its own. `bilbo derive` writes nothing for a forgotten entry, and writes no notes or summary again that were forgotten while no entry holds the capture label they had.

### A forgotten file

The entry's file is left where it is. bilbo takes a file for a forgotten entry's only when no entry holds the file's own path and the file is of that entry's kind. When it reads the file, it goes by the `id` the file carries or, when no entry holds an `id` of the file's, by its path and the bytes it begins with. So a file with no `id` that was renamed, or whose beginning was edited, can be kept again, as a new entry.

A pass over the follows, and `bilbo remember <folder>`, pass such a file over: nothing of it is kept, and it counts in `unchanged`. A run that read the file and ends without a store failure says `passed over <path>: its entry <id> was forgotten`, and adds `; bilbo remember <path> keeps it again`, the path written for a shell, unless the path holds a character a line would not show as it is.

`bilbo remember <file>`, with such a file itself as its target, is the way back. Unless the file is refused or the store fails, the entry has the file's capture label again and the file is kept into it as any kept entry's file is; a label `derive:<id>` or `summary:<id>` that a forgotten entry had and no entry holds goes back to such an entry, as it is, and `bilbo derive` judges the entry as any other; and a run that ends without a store failure says `kept <path> again: its entry <id> was forgotten`.

### Output

Standard output, one line: `forgotten <n>, belonging <n>, unchanged <n>`: the entries named that this run forgot, those it forgot because they belong to an entry named, and the entries named that were forgotten already. A run that ends without a store failure also says a line on stderr for each entry it forgot, beginning `forgot <id>`, with the label it had.

When the store fails while the run writes, the run exits 1 with no counts and no `forgot` line, and says, before the error, `stopped part way: bilbo forget <id>... finishes it`, with the id of each entry named. Run that command once the cause is gone, not the one you typed.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when every entry named was forgotten already |
| 1 | refused or failed, as for a ref that names no entry |
| 2 | usage or config error, as for a ref that names several entries |

## reindex

```text
bilbo reindex
```

Makes the catalog and the search index again from the store. Both live in the cache folder ([Configuration](configuration.md#folders)) and hold nothing the store does not, so `reindex` loses nothing: run it after the cache folder was deleted or restored from a copy, after `search.chunk_tokens`, `search.chunk_overlap` or `markdown.heading_width` changed, or when an index is in doubt.

### What it makes

- **The catalog**, `catalog.sqlite`: rebuilt from every segment and `open.seg`, as a `bilbo remember` rebuilds a missing one.
- **The label tables**, in `catalog.sqlite`: the text labels of every entry the run read and indexed (for a session, the folders its records name), and their words, which `recall --in` matches. Only an index run files them: a catalog made again holds none until the next `remember`, `refresh`, `forget` or `reindex`, and nothing takes a row from an entry the committed index still holds. The capture label and the id label are in the store's own meta and survive any rebuild.
- **The index**, the folder `index/`: one document for each chunk of each entry's newest generation. A chunk is consecutive records packed up to `search.chunk_tokens` tokens of the Qwen3-Embedding tokenizer; a record longer than that is cut into chunks that share `search.chunk_overlap` tokens. A record that renders to more than 1 MiB is cut at line ends before it is counted. The index holds each chunk's words (lowercased, without 33 English stopwords, stemmed) and where each stands in the chunk, and where its records are in the store (entry, generation, record range, date, kind), its length and the digest of its text, never its text. A word of more than 4,096 bytes is not indexed.

An entry whose kind no reader reads is left out and counted as unread. Older generations of an entry are not indexed; `bilbo read` still prints them.

`reindex` replaces only its own files: `catalog.sqlite` with its journals, and the files of an index it made. The `index/` folder holds a file `bilbo-index` that marks it as bilbo's. An `index/` folder that holds anything and lacks that file, that holds a stranger beside it, or that is a link is refused, exit 1, and nothing in it is touched: move it away or set `paths.cache` to another folder. The old index stays in place until the new one is whole, so while `reindex` runs the folder holds both.

### What it keeps

The catalog remembers the size and time of each file `remember` kept, so a later run skips a file that has neither changed. `reindex` carries those over from the old catalog. When the old catalog is missing, unreadable or another store's, they are lost, and the first `bilbo refresh` after it reads every followed file once more to find that nothing changed.

### While another bilbo runs

`reindex` holds the store's lock from start to end, as a `bilbo remember` does, and waits for one that is writing, saying so on stderr. A `bilbo remember` waits for `reindex`. A `bilbo refresh` that finds `reindex` running exits 0 and asks for one more pass, as it asks any writer ([Two runs at once](#two-runs-at-once)). `reindex` closes as `remember` does: once it has committed the index it serves that request by running the follows, grows the index from the cursor it committed after each pass, and seals when a seal is due. Its lines for a pass are `remember`'s, on stderr; stdout counts only its own rebuild; the exit code is 1 also when a pass failed or refused a file. A `reindex` whose own rebuild fails releases the store as it is: it runs no close loop, does not seal and leaves another bilbo's request for the next writer. Only a failure inside its close loop follows `remember`'s rule that the run tries the index no more.

### When it is needed

`remember` and `refresh` keep the index current, so `reindex` is for an index in doubt, and after a store is restored from a copy, always: bilbo cannot tell an older or branched history that kept its catalog id from a current one. An index kept that way ranks as a rebuilt one, scores included. An index of an older format is made again by the next `remember`, `refresh`, `forget` or `reindex`: this bilbo folds accents, keeps where each word stands and labels a session with the folders it was given, so the first one after an upgrade makes the catalog, the labels and the index again, once. A failed entry is tried again by the next two runs of `remember`, `refresh` or `forget`, then waits for a change or a `reindex`.

### Output

Standard output, two lines:

```text
entries <n>, chunks <n>, unread <n>, failed <n>
index bytes <n>, milliseconds <n>
```

`entries` are the entries indexed, `unread` the ones left out for their kind, `failed` the ones left out whole because their records could not be read back, a reader or the tokenizer failed on them, or the tokenizer could not split a text of them; each kind left out and each failed entry gets a line on stderr, and any failed entry makes the exit code 1. `index bytes` is the size of the index's files, `milliseconds` how long the run took. On a terminal the same counts come as a marked message. An entry that holds summaries ([Summaries](#summaries)) is of a kind bilbo keeps with no reader, `summary`: every index run passes over it, and it counts in none of the four and has no line.

A store that cannot be read, or a damaged frame found while the catalog is rebuilt, stops `reindex` with the store's error and exit 1 before the index is touched.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when entries of a kind no reader reads were left out |
| 1 | an entry could not be read back, the `index/` folder is not bilbo's, or the store or the index could not be read or written |
| 2 | usage or config error, a store root that holds bilbo 0.20's `notes/` or `library/`, or a state or cache folder inside the root |

## derive

```text
bilbo derive [--dry-run] [--redo] [--calls <n>] [<entry>...]
```

Writes notes about entries that stopped growing, with a model you configure, and keeps them as entries of their own. With no `<entry>` it takes every entry `derive.in` names that is due; with some it takes those alone. An `<entry>` is an id, the end of an id or a session's id, as `bilbo read` takes one, with no records after it: the page has no room for that. A ref that names records, `#r3-9` or `#g1`, is a usage error.

Whatever the kind and whatever the instruction, what derive writes is notes: paragraphs that each point at a range of records of the entry they were written from. An instruction changes what the paragraphs say, not what they are. A run also writes a summary of each entry: one short paragraph about the whole entry, which `bilbo digest` shows on the entry's row ([Summaries](#summaries)).

### Turning it on

Three keys must be set before anything is sent: `derive.enabled`, `llm.provider` and `derive.in`. No bilbo verb but `derive` sends anything, and `derive` sends nothing while one of the three is unset. For a provider reached by its API:

```sh
bilbo config set llm.provider anthropic
bilbo config set llm.anthropic.model claude-haiku-5-5
bilbo config set derive.in '["<a word of a label, or an id>"]'
bilbo derive --dry-run
bilbo config set derive.enabled true
```

For the words of a label, `derive.in` and `derive.not_in` follow the rules of `recall --in`: both reach a session by the folders it worked in and by the folders it was given.

`derive.in` also names the entries a summary may be written of, and that is every kind: with the instructions bilbo ships, a Markdown file that `derive.in` names can be sent for its summary though no note is written of it ([Summaries](#summaries) says when). A kind you give an instruction in `derive.instructions` gets notes, and its summary is then written of them. A word that names sessions by their folder names the files kept from that folder too. The dry run's `summaries:` line counts what is due before anything is sent, and `derive.summary.enabled = false` asks for none.

Run `--dry-run` before `derive.enabled`: it says what a run would send, and sends nothing, whether derive is on or not. The providers are `anthropic` and `openai`. A provider is reached by the way its `via` names: `api`, a request with a key, unless you set another. `bilbo config set llm.anthropic.via tool` takes Anthropic's tool, `claude`, with its own login, and `bilbo config set llm.openai.via tool` OpenAI's, `codex`, with a login of its own in a folder of bilbo's. The dry run's `provider:` line says the way.

### Providers

A provider is who makes the models, and `llm.<provider>.via` is the way bilbo reaches them: `api` sends a request to the provider's API with a key, `tool` starts the provider's tool with its own login. Both providers take both. The provider's table, `[llm.anthropic]` or `[llm.openai]`, holds `model` and `via`. `model` has no default, and both ways read it. Each way has a table of its own under it, and only the one `via` names is used; a value in the other is still checked. Each key is in the [configuration page](configuration.md#keys).

By `api` a provider needs a `model` and a key. `[llm.anthropic.api]` holds `endpoint`, `version`, `key_env`, `key_command`, `headers` and `extra`; `[llm.openai.api]` holds the same without `version`. `extra` adds fields to the body of every request, and `headers` adds headers:

```toml
[llm.anthropic]
model = "claude-haiku-5-5"

[llm.anthropic.api]
extra = { output_config = { effort = "low" } }
```

```toml
# Refusals of a safeguard answered by another model, on the provider's side.
# The header and the field are the provider's; bilbo turns nothing of it on.
[llm.anthropic]
model = "claude-haiku-5-5"

[llm.anthropic.api]
headers = { anthropic-beta = "server-side-fallback-2026-07-01" }
extra = { fallbacks = "default" }
```

bilbo refuses a field of `extra` that it writes itself, that offers the model a tool or that makes the provider keep the records, and a header that it writes itself. `extra` may ask the provider to cache the prompt.

With the second the model that answered can differ from the one named; the log has both. `derive.answer_tokens` counts the model's thinking, and a low value is spent on thinking first: an answer cut there holds no note.

By `tool`, `anthropic` is the tool claude, with its own login and no key: `[llm.anthropic.tool]` holds `program`, `effort`, `arguments`, `inherit`, `environment` and `project_files`. bilbo runs the tool with no tool the model can call, and does not start it otherwise: `llm.anthropic.tool.arguments` is read as flags bilbo knows, each with its values: `-p`, `--model`, `--effort`, `--output-format`, `--tools`, `--disable-slash-commands`, `--strict-mcp-config`, `--setting-sources`, `--settings`, `--no-session-persistence`, `--safe-mode`, `--system-prompt`, `--append-system-prompt`, `--fallback-model` and `--max-budget-usd`. Any other item is refused, and so is a flag written with `=`. The list must hold `--tools` followed by `""`, `--strict-mcp-config` and `--safe-mode`, and `{instruction}` only as the value of `--system-prompt` or `--append-system-prompt`. A flag of a later version of the tool waits for bilbo to know it. Two things no argument rules: what `llm.anthropic.tool.environment` or a settings value makes the tool do, and the hooks of the machine's managed settings, which the tool runs with the prompt whatever its arguments are. A reply that shows a tool was reached is a failed call: more than one turn, a denied permission, or a count above 0 of a server tool or of a subagent. A reply without one of the fields bilbo reads for that is a failed call too, and charges nothing: bilbo reads the reply as claude 2.1.327 prints it, and the run names the field. The list must also hold `--no-session-persistence`: without it the tool would save every call as a session under `~/.claude/projects`, which `bilbo refresh` follows by default.

The tool runs in an empty folder of bilbo's state folder, `<state>/derive/work`, with only the variables `llm.anthropic.tool.inherit` names: its own login is found through `HOME`, and nothing of the caller's session reaches it. bilbo does not start it when a folder above that one holds a file the tool reads as a project's (`llm.anthropic.tool.project_files`): set `paths.state` outside any project. `DISABLE_PROMPT_CACHING` is set because the tool would otherwise pay to cache a prompt it never sends twice. bilbo stops the tool at `llm.timeout`, with what it started in its process group; a process that starts a session of its own is not stopped. A tool may make more than one request of its own for one call; with no tool on offer it makes one. A timer's run has no shell environment: give `llm.anthropic.tool.program` its full path.

By `tool`, `openai` is the tool codex, release 0.169.0, on a ChatGPT login of its own and no key: `[llm.openai.tool]` holds `program`, `effort`, `arguments`, `inherit`, `environment`, `home` and `system_files`. It is for a personal ChatGPT plan. bilbo runs the tool with no tool the model can call, and does not start it otherwise. bilbo finds the program once a run, asks it its release then and again before every call, and stops at any other than 0.169.0: its arguments, and its reading of what the tool prints, are that release's, and a later one waits for bilbo. It starts the program by the path it found it at, for every call, and stops the run if that path leads to another file meanwhile. bilbo writes the model's entry of the tool's own catalog, as `codex debug models --bundled` prints it, with its tools turned off: `llm.openai.model` must be a model that release bundles, and `llm.openai.tool.effort` an effort that entry lists. `llm.openai.tool.arguments` is read as flags, settings and features bilbo knows: `exec` first and `-` last; the flags `--json`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, `--skip-git-repo-check`, `--strict-config`, `-s`, `-m`, `-c` and `--disable`; a setting of `-c` or a feature of `--disable` that is not in the default list is refused, but for `model_verbosity`, `model_reasoning_summary` and `service_tier`, which `-c` may add. The list must keep every item of the default that removes a tool or your own setup, with `-m` followed by `{model}` and the effort as `{effort}`, and the run names the one that is missing. A reply that shows a tool was reached ends the run, and charges no entry a try.

The tool gets a home of bilbo's own, `<state>/derive/codex` unless `llm.openai.tool.home` names another, so nothing of your codex setup reaches a request. bilbo never makes it and never opens its login. Make it and log in once:

```sh
mkdir -p -m 700 ~/.local/state/bilbo/derive/codex
CODEX_HOME=~/.local/state/bilbo/derive/codex codex login
```

Give `CODEX_HOME` to that one command and do not export it. `bilbo derive --dry-run` prints the folder in force as `home:`. Use it for nothing else. bilbo does not start the tool when the folder holds a file codex reads as your setup (`AGENTS.md`, `AGENTS.override.md`, `config.toml`, `.env`, `environments.toml`, `rules`, `skills`, `hooks.json`, `agents`, `memories`); when it is your own codex home or the folder the tool runs in, or holds one, or is inside one; when its group or others can write to it; or when its `auth.json` is missing, a link, or a file with a second name.

The tool runs in the folder claude runs in, `<state>/derive/work`, with only the variables `llm.openai.tool.inherit` names, the ones `llm.openai.tool.environment` gives, and two bilbo sets: `CODEX_HOME`, and `HOME`, which is that empty folder, so the tool finds nothing of yours through it. A git checkout around that folder adds nothing: the arguments must hold `-c project_root_markers=[]`, with which codex takes the folder it runs in as the project's root and reads no skill of a checkout above it. bilbo itself needs `HOME` or `CODEX_HOME` in its own environment, to tell your codex home from its own. Neither list takes `HOME` or a variable whose name starts with `CODEX_` or `OPENAI_`: such a variable can give the tool another login or another place to send a request. A machine that reaches the network through a proxy adds `HTTPS_PROXY` to `inherit`. `derive.answer_tokens` is not sent, since the tool has no setting for it: an answer the service cuts is a failed call, and the run says to lower `derive.window`. The log names the model that answered only when the service sent the request to another. A spent plan or a lost login stops the run, and so does an error of the tool's that bilbo does not know: it charges no entry a try. A timer's run has no shell environment: give `llm.openai.tool.program` its full path.

Three things no argument rules. A file of the machine that configures codex whatever its arguments are: bilbo does not start the tool while one of `llm.openai.tool.system_files` exists (`/etc/codex/config.toml`, `/etc/codex/managed_config.toml`, `/etc/codex/requirements.toml`, the machine's skills in `/etc/codex/skills`, and on macOS the managed preferences of the machine or of one of its users, where Apple documents them); read the file, then take it out of the list to run with it. A file or a folder there that bilbo cannot read stops the run too. A forced preference macOS keeps anywhere else is not seen. A configuration bilbo cannot see: the one a business, education or enterprise workspace of ChatGPT gives its members from the service, which is why this way is for a personal plan. And what codex adds on the account's word: while the account's Codex setting for commit attribution is on (the service reports it to the tool as `commit_attribution_enabled`), every request carries codex's instructions for commits and pull requests beside bilbo's. bilbo can neither remove nor see them, and the model has no tool to act on them; turn the setting off in the account to keep them out.

### The key

An API key is never a value of the config file: `bilbo config` prints every value. Name the variable that holds it with `key_env`, or a program that prints it with `key_command`, both in the provider's `api` table; the program's output is in no message. The program is your own: it gets bilbo's environment as it is and runs in bilbo's state folder, within `llm.timeout`. An endpoint is printed by `bilbo config` and in errors like any value, so no key belongs in it. When a run has read the key, a setting that goes into the request and holds it (`model`, or `version`, `headers` or `extra` of the `api` table) stops the run before anything is sent, and the message names the setting, never its value; the endpoint is the exception, which the guard does not read, so no key belongs in it.

A timer's run has no shell environment, so a variable your shell sets is not there. Give the key through `key_command`, with the program's full path. With systemd the unit can instead name a file of your own with `EnvironmentFile=`. Make that file private before the key goes into it:

```sh
install -m 600 /dev/null <the file>
```

Then write one line in it with an editor, the variable `key_env` names, `=`, and the key (`ANTHROPIC_API_KEY=<the key>` with the default), and add `EnvironmentFile=<the file>` to the unit. Never write the key into the unit's file or the agent's plist. A launchd agent can name no such file, so on macOS `key_command` is the one private way: a program that reads the key from your keychain.

### Where a request goes

An `https` endpoint follows the proxy the environment names, as other programs do; the tunnel keeps the request encrypted to the provider. A plain `http` endpoint is taken only at `127.0.0.1` or `[::1]`, written as an address and not as `localhost`, and is reached with no proxy: the key is in the request. No redirect is followed.

### What is due

Each entry is in one state, the first that holds: its labels are not current (the index failed on it, or on the entry it belongs to); its kind has no instruction; it belongs to another entry, as a subagent's session does; `derive.in` does not name it; `derive.not_in` does; it is still growing; it is covered; it is passed over; or it is due. An entry is still growing until its newest records were sealed `derive.quiet` ago, so a note appears one to two hours after an entry's last record with the defaults; naming the entry skips that test and no other. An entry is covered when the runs of its newest notes cover every record, whichever instruction, provider or model wrote them: only `--redo` compares those with the ones in force. An entry whose kind's reader cannot promise that new records leave earlier text as it was (Markdown) is derived again from its first record each time it grows: an instruction added for such a kind costs a full pass at every change of the file. `derive.in` and `derive.not_in` are read when a `derive` run plans. A tag you add or remove while a run is under way counts from the next run: to keep an entry from a provider, tag it before the run that would send it. A summary is judged apart from the notes: an entry whose kind has no instruction is left out of note writing and can still be due for its summary ([Summaries](#summaries)). Nothing is planned for a forgotten entry, nor notes for an entry whose notes were forgotten while no entry holds their label ([What belongs to it](#what-belongs-to-it)).

### A call

An entry's uncovered records are cut into windows of at most `derive.window` tokens, of whole passages; a passage over `derive.passage` tokens is sent as its start and its end. `derive.window` counts the records' text alone: the request is that plus the instruction and up to `derive.earlier` tokens of the entry's newest earlier notes. Windows are called in order, and a window is called only when every record before it is covered. `derive.calls` counts requests sent to an API and runs of a tool's program, retries included. At a replay, an answer whose entry is at another generation than the one it was asked about is dropped, with a line.

### When a call does not give notes

What bilbo does with a call that gave no note depends on whose the failure is.

- **Refused:** the provider declined. The window is charged at once, its entry waits for the next run, and the run goes on.
- **The window's own:** the answer held no note bilbo can keep, reached a tool, or held the key. The window is charged at once, its entry waits for the next run, and the run goes on. With the tool codex a reached tool is not the window's: it ends the run and charges nothing.
- **The provider's or the settings':** the provider is busy, limits the rate, had an error of its own or could not be reached; or the key, the credit, a permission, the model or the address is wrong; or the connection could not be made secure; or its program could not be started. A busy provider is asked again, up to `derive.retries` times, after the wait it named or `derive.retry_wait`, multiplied by `derive.retry_factor` at each retry after the first; a wait over `derive.retry_limit` ends the run. Otherwise the run ends at once. Nothing is charged.
- **Either's:** no answer within `llm.timeout` after the retries, a request the provider rejected, an answer bilbo cannot read or one over `llm.response`, an answer that was cut or ended for a reason bilbo does not take, a tool's error. The entry waits for the next run and the run goes on. The window is charged once another call of the same run to the same model is answered, which shows that model was working: when `derive.tags.model` names a different model, an answer of the model that tags proves nothing of the one that writes the notes, nor the other way round. `derive.failures` such calls to one model in a row, with no answer of that model between them, end the run. Any answered call of a run to that model is the proof, so a setting that fails only the longest windows passes them over, and `--redo` takes them again once the setting is mended. A setting that makes every call give a cut answer or an unknown stop reason costs `derive.failures` calls a run and charges nothing. With `derive.calls` no larger than `derive.retries` plus one, a window that is always busy or slow is called at every run and never passed over: the calls end the run before its retries do, and nothing is charged. With the tool codex an answer bilbo cannot read and an error of the tool's that bilbo does not know are not either's: each ends the run and charges nothing.

A run with a failed call exits 1. The newest entry comes first. A run that made a call and neither answered nor charged begins the next run one entry further, so no entry that fails keeps the others from their call. A run that ends by itself or at `derive.calls` charges the failures that still wait for each model that answered in it, and for no other; a run the provider ended leaves them all uncharged. A run that is killed between an answer and the charges that answer proves loses those charges, never the answer: the windows are called once more and charged at the next proof. A pass's record names no model.

A window charged `derive.tries` times is passed over: a run record says so, with no call, and the entry goes on. A request the provider rejects is said with its status, the provider's own error identifiers and the request's id, never with the provider's sentence: an error can hold text of the entry.

### Tags

With `derive.tags.enabled`, a run also files each entry it has written notes for under tags, with one more call an entry. It is off unless you turn it on, and the three keys of "Turning it on" still decide whether anything is sent at all.

```sh
bilbo config set derive.tags.pick '["bilbo", "garden"]'
bilbo config set derive.tags.enabled true
```

The model picks from `derive.tags.pick` and from nothing else: a name it writes that is not on the list is dropped, and with an empty list no call is made. A name on the list need not be a tag any entry carries yet. The call is sent the entry's notes, the newest that fit `derive.tags.notes` tokens and the newest one whatever its size, and the list. bilbo adds nothing of the entry to them, no id, no path and no label, and never sends the entry's records here; the notes hold whatever names and paths they noted. The call goes to the provider and by the way `[llm]` names, to the model `derive.tags.model` names, else to the one that writes the notes. An answer gives an entry at most `derive.tags.most` tags, and none that would leave it with more than `tag.cap`.

The call's instruction is one bilbo ships. `derive.tags.instruction` names a file whose text is sent in its place, as `derive.instructions` does for a kind's notes. An instruction changes which tags are picked, not what an answer is: the file's text has to ask for the answer bilbo reads, each tag on a line of its own, exactly as it stands in the list, or the one line `(none)` when no tag fits. Whatever the file says, a name that is not on the list is never declared. What follows the instruction does not change: the notes inside `<notes>`, the list inside `<tags>`, one name a line, and a last line that asks for the tags. The file is read only while the tags are on and the list holds a name. A run that the three keys of "Turning it on" stop does not read it; a dry run is not stopped by them. When it is read and is missing, empty or not UTF-8, the run ends with a config error, exit 2. Another text is another instruction. By itself it asks no settled entry again, one that has an answer about every name of the list and no note since: `--redo` makes every entry whose last answer another instruction gave due again. A call for tags that a run reaches is made under the text in force, for an entry due for a new note or a name it has no answer about, and for one whose asking was passed over under another text: that pass was of another asking.

An entry's tags are asked for once its notes cover every record, and again when a note is added to it or a name it has no answer about joins the list. They are declared as `bilbo tag <entry> --add` declares them, and `bilbo read` and `bilbo tag` show them as a model's. A tag you declared is never taken away, and a name you removed from an entry is never declared for it again. `--redo` asks again for an entry whose tags were picked under another instruction, provider or model, or were passed over, and takes back the model's tags that the new answer does not pick.

A call for tags that is refused or fails is treated as a window's: after `derive.tries` charges that asking is passed over. The notes written before it are kept. By the tool `claude`, `llm.anthropic.tool.arguments` must give `--model` the value `{model}`, hold no `--fallback-model`, and give a system prompt no other value than `{instruction}`; the default does. A list that does not ends the run before its first call for tags, with the notes kept. A run whose calls end before that call is not stopped for it.

Only an entry `derive` wrote notes for gets a tag this way. An entry `derive.in` does not name, one `derive.not_in` names, one still growing that you did not name on the command line, and one whose kind has no instruction get none. So `--in tag:<name>` names the part of the store that was tagged and says nothing of the rest: add the words or the id that name the rest. A session's subagents and its notes belong to it and come with it. A `derive.in` that names nothing but a tag the model itself declares sends nothing until you declare that tag for an entry yourself: the model only tags what `derive.in` already names.

bilbo keeps what each entry was answered in a file of its state folder, `derive/tagged`. On another machine, or if that file alone is lost, each entry is asked once more. An ordinary run takes no tag away by it: the new answer can only give the entry a tag of the model's that it did not carry. A run with `--redo` asks each such entry again as well and, as `--redo` does, takes back the model's tags the new answer does not pick. Either way a tag you declared stays, and a name you removed does not come back. The state folder also holds what the store has not sealed yet: losing the whole folder can lose your latest changes of tags with it, as it loses any capture not yet sealed.

Turned off again, a run asks for no tag. The tags already declared stay and still count, for `--in` and for `derive.in` alike, and an answer an earlier run had already been given is still kept.

### Summaries

With `derive.summary.enabled`, which is on unless you turn it off, a run also writes a summary of each entry it may send: one paragraph that says what the entry is about and what it holds, in the one or two sentences the instruction bilbo ships asks for, for a reader who sees it on a row of [`bilbo digest`](#digest) and decides whether to open the entry. The three keys of "Turning it on" still decide whether anything is sent at all.

A summary is written of an entry that `derive.in` names and `derive.not_in` does not, once it stopped growing, whatever its kind; naming the entry on the command line skips that wait and no other test. A forgotten entry gets none, and neither does an entry whose summary was forgotten while no entry holds `summary:<id>`. An entry that belongs to another, as a subagent's session or the notes of a session do, gets none. Where bilbo has an instruction for the entry's kind, the summary is written of the entry's notes, once note writing has nothing left to write of the entry, and the call is sent those notes. Where it has none, as for a Markdown file or a note kept with [`bilbo remember -`](#a-note-with-no-file), it is written of the entry's own text, and the call is sent that text. bilbo adds nothing of the entry to either, no id, no path and no label. What is sent of an entry is cut to `derive.summary.window` tokens: its start and its end, shared by `derive.passage_head`, around a line `[cut: <n> tokens]`. An entry whose notes or text are fewer than `derive.summary.least` tokens gets no first summary: it is short enough to read as it is. The call goes to the provider and by the way `[llm]` names, to the model that writes the notes: one call an entry, after the entry's notes and its tags.

An `<entry>` you name whose kind has no instruction is taken for its summary, by the same rules: one that `derive.in` does not name is refused as any other ref is. The run writes the summary when it is due, says nothing of it when it is written, and exits 0. With `derive.summary.enabled = false` such a ref is refused, since its kind has no instruction, and the run exits 1.

The call's instruction is one bilbo ships, the same for every kind. `derive.summary.instruction` names a file whose text is sent in its place; the text has to ask for the answer bilbo reads, one paragraph of plain text. What follows the instruction does not change: the notes inside `<notes>` or the text inside `<entry>`, and a last line that asks for the summary in at most `derive.summary.most` characters. A longer answer is kept whole. An answer is not kept when it is empty, when it is more than one paragraph, when the share of its names that stand in what was sent is under `derive.names`, or when it is one string the store would replace by a placeholder ([Opaque strings](#opaque-strings)): that is a failed call.

A call for a summary that is refused or fails is treated as a window's ([When a call does not give notes](#when-a-call-does-not-give-notes)): after `derive.tries` charges that asking is passed over, until `--redo` asks again or the asking is another one. It is another when what would be sent changed, and also when the entry's records, the provider, the model or the instruction did, though the same text would be sent. The notes and the tags written before it are kept. By the tool `claude`, a summary needs of `llm.anthropic.tool.arguments` what a call for tags needs ([Tags](#tags)), and the default gives it. A list that does not bind asks for no summary and by itself stops no run: the run says so once on stderr, `no summary is asked in this run: ` and what the list needs, and writes its notes as it did. A call for tags with such a list still ends the run wherever the run reaches it, as [Tags](#tags) says.

A summary is kept in the store, in an entry of its own under the capture label `summary:<the entry's id>`, one record for each answer bilbo keeps; the last record is the summary in force. That entry is of the kind `summary`, which no reader reads, and it belongs to no entry: it is in no index, `bilbo recall` never finds it, an `--in` that names its entry or a word does not take it in, and no list of `bilbo digest` ranks it. Named by its own id, or tagged by hand, it is an entry as any other. Every run that brings the index up to date passes over it and says nothing of it ([`bilbo reindex`](#reindex)). [`bilbo read`](#the-outline) prints the summary in its entry's header; a read of the summary's own entry says that its kind cannot be read. A summary is written again when what it was written of changed: a file's records, or a session's notes. A key that changes, `derive.summary.window`, `derive.summary.most` or `derive.summary.least`, writes none again. Until it is written again, `bilbo digest` shows the one it has: a model's words about the entry as it was when this summary was written.

When nothing is left to summarise, a file that is gone, emptied or left with white space alone, or a session whose notes cover every record and hold no note, a run that takes the entry retires its summary: it appends a record that is none, and no row and no header shows one. A run takes an entry for that as it takes one for a summary: `derive.in` names it, `derive.not_in` does not, and it stopped growing or the run names it. So a file withdrawn a moment ago waits until its records are sealed `derive.quiet` ago, and a run that names it retires its summary at once. A `--redo` that stopped part way retires nothing: the summary it found is still about the same records. An entry `derive.in` no longer names keeps the summary it has. A forgotten entry's summary is not retired.

bilbo keeps the charges of a call that is failing, and the asking that was passed over, in a file of its state folder, `derive/summaries`. What was written is in the store, so on another machine, or if that file is lost, no summary is asked again while what it was written of is the same, unless `--redo` asks. An asking that was passed over is asked again then, and its charges begin again.

Turned off again, a run asks for no summary. The summaries already kept stay, and `bilbo digest` shows them until `digest.summary.enabled` is `false`: the first key stops the calls, the second hides the summary field on the digest's rows.

### --redo

`--redo` makes due every entry whose newest notes were written under another instruction, provider or model than the ones in force, and every entry with a stretch that was passed over, as a new generation of its notes from its first record. It never writes again what the instruction, provider and model in force wrote in full. The way is no part of this: the same model of the same provider, reached by `tool` after `api`, is the same writer, and nothing is redone. With `derive.tags.enabled`, it also asks again for the tags of an entry whose last answer was under another instruction, provider or model, or whose asking was passed over. With `derive.summary.enabled`, it also writes again the summary of an entry whose last one was written under another instruction, provider or model, and asks again an asking that was passed over.

### --dry-run

A dry run builds the same plan and prints it, and does nothing else: it takes no lock of its own (it reads the store as `recall` does), starts no program, writes no file and calls no provider; of the key it says only whether its variable is set. Its lines, each absent when it has nothing to say:

```text
off: derive.enabled is false; a run would send nothing
kinds: session (built in, 9f86d081884c7d65)
named: 412 entries by derive.in, 14 of them left out by derive.not_in
due: 37 entries
not due: 12 still growing, 340 covered, 9 belong to another entry, 2 passed over, 1 with labels not current
this run: 20 calls for 14 entries, 380122 tokens of records to send (derive.calls 20)
provider: anthropic, via api, model claude-haiku-5-5, at https://api.anthropic.com/v1/messages
version: 2026-10-21
key: the variable ANTHROPIC_API_KEY, which is set
```

`headers:` and `extra:` follow when they are set. By the way `tool` the lines after `this run:` are `provider: anthropic, via tool, model <model>, effort <effort>, program <program>`, then `arguments:`, `inherit:`, `environment:` and `project_files:`, each with its value.

With refs, every count is of the refs alone, and each ref has one more line, `entry`, in five fields apart by tabs: the entry's id, its state (`due`, `covered`, `passed over` or `left out: ` and why), the id of its note entry or `-`, and how many notes its newest generation holds. With `derive.tags.enabled`, a line that starts `tags:` follows `this run:`, and each ref has a `tags` line after its `entry` line; the walk that counts this run's calls gives the calls for tags their turn. The `tags:` line names the instruction in force as the `kinds:` line names a kind's: `instruction (built in, <id>)`, or `instruction (<file>, <id>)` when `derive.tags.instruction` names a file. With an empty list the line is `tags: none to pick from: derive.tags.pick is empty` and names no instruction: the file is not read then.

With `derive.summary.enabled`, a line that starts `summaries:` follows `this run:` and `tags:`, and each ref has a `summary` line after the `tags` lines: `summary`, the entry's id and its state, apart by tabs.

```text
summaries: 731 entries due; this run 6 calls, 14210 tokens to send; instruction (built in, 7fdf9f0d93f443e3)
summary	01M6ZGE4KTBYYH07AS2KVMSCAF	written
```

The lines above it stay about notes: an entry of a kind with no instruction counts in none of them and its `entry` line says `left out: no instruction`, unless its labels are not current, which is tested first: then it counts in `not due:` and its `entry` line says `left out: labels not current`. Its `summary` line says what its summary is. That state is `due`, `written`, `passed over`, `not due: notes not complete`, `not due: no note`, `not due: short`, `not due: no reader reads its kind`, `not due: forgotten`, or `not due: ` and why the entry is left out or `still growing`. The walk gives a summary's call its turn after the entry's windows and its tags; for an entry whose windows are still to be called it counts the call and no token, since what will be sent is not written yet. With a list of `llm.anthropic.tool.arguments` that does not bind a call to its model ([Tags](#tags)) it counts no call for any summary, since the run asks for none. The line names the instruction in force as the `tags:` line does.

### The timer

A timer runs `bilbo derive` every 30 minutes. Nothing installs it yet: its files are in [`docs/reference/timer/`](timer/), with `@BILBO@` for the absolute path of `bilbo` and `@LOG@` for the log file. Run these from a checkout of the bilbo repository.

The timer sets no variable: a unit installed as written reads the default config file and the default folders, not the ones your shell's `BILBO_*` or `XDG_*` variables name. Set `llm.*`, `derive.*` and `paths.*` in the config file.

On macOS, with launchd:

```sh
mkdir -p ~/Library/LaunchAgents ~/Library/Logs
sed -e "s|@BILBO@|$(command -v bilbo)|" -e "s|@LOG@|$HOME/Library/Logs/bilbo-derive.log|" \
  docs/reference/timer/io.github.delucca.bilbo.derive.plist \
  > ~/Library/LaunchAgents/io.github.delucca.bilbo.derive.plist
launchctl bootstrap "gui/$(id -u)" ~/Library/LaunchAgents/io.github.delucca.bilbo.derive.plist
```

On Linux, with systemd:

```sh
mkdir -p ~/.config/systemd/user ~/.cache
sed -e "s|@BILBO@|$(command -v bilbo)|" -e "s|@LOG@|$HOME/.cache/bilbo-derive.log|" \
  docs/reference/timer/bilbo-derive.service > ~/.config/systemd/user/bilbo-derive.service
cp docs/reference/timer/bilbo-derive.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now bilbo-derive.timer
```

### Files

`<state>/derive/` holds seven paths: `lock` (the run's lock, held to its end), `spool/` (the blocks waiting for a replay), `failed` (the windows charged, one line each, and one line that marks the entry the next run begins after), `log` (one line for each attempt, mode 600) and `work/` (where the tool runs). The log holds no text of a record. bilbo refuses the lock, a block and the file of charges when it is a link, has a second name, is no regular file or is not yours alone; the folders are not checked. A line of `failed` goes when its window is answered or passed over, and when its entry's windows have moved on. If a run dies between keeping its notes and bringing the index up to date, the notes are in the store and the next `bilbo refresh` indexes them; until then `bilbo derive` says the index is behind and sends nothing. It also holds `tagged`: what each entry was answered about tags, the asking that was passed over, and the charges of the asking that is failing. The log has a second line form, `t1`, for a call for tags. And `summaries`: the charges of a call for a summary that is failing and the asking that was passed over. An answer for a summary waits in `spool/` under a name of its own, `<number>.summary`, which a bilbo from before the summaries leaves where it is. The log has a third line form, `u1`, for a summary: one line for each attempt of a call, and one with no call behind it whose third field is `passed`, for an asking the run decided to pass over at `derive.tries` charges, or `retired`, for a summary it decided to retire. Such a line is written when the decision is kept in `spool/`: the charges' file, `derive/summaries`, or the store takes it at the replay, which drops it when the entry changed in between.

### Output

Standard output, two lines:

```text
entries <n>, calls <n>, notes <n>, dropped <n>, refused <n>, failed <n>
tokens in <n>, tokens out <n>, seconds <n>
```

`entries` are the entries a call was made for, `calls` every attempt, `notes` the paragraphs kept, `dropped` the paragraphs a check dropped, `refused` the calls the provider declined and `failed` the failed calls, the cut answers and the answers with no note kept. Each refusal, each failed call and each window passed over has a line on stderr. A run that is off, that finds another derive running, or that cannot trust the labels prints nothing on standard output. On a terminal the two lines come as one marked message. A call for tags counts in `entries`, `calls`, `refused`, `failed` and the tokens; `notes` and `dropped` are of notes alone. A call for a summary counts as a call for tags does. A run that kept summaries says `summarised <n> entries` on stderr.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | done, also when derive is off, nothing is due or another derive runs; a second derive that finds the store held longer than `store.wait` exits 1 with `another bilbo is writing the store` |
| 1 | a failed call, a ref that is refused, a provider that stays busy or stops the run, labels that cannot be trusted, a store or index failure, or a file refused while derive ran the follows again for another bilbo |
| 2 | usage or config error |
