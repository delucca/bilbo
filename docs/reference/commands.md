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
```

Keeps files as entries in the store. An entry is a sequence of records, numbered from 1 per generation. Run it again and it keeps only what is new. With a target, it keeps that file or folder; with none, it keeps every follow.

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

### The index

A run also brings the search index to what the store holds, inside the lock that kept the files and before it releases: it indexes the entries that changed since the cursor the index's last commit recorded, and an entry whose kind cannot promise that appending records changes no passage already indexed (Markdown) is indexed again whole. When the index cannot be continued, because it was never made or is broken, `search.chunk_tokens`, `search.chunk_overlap`, `markdown.heading_width` or a rule of this bilbo changed, or the catalog was rebuilt, the run makes it again in full, as [`reindex`](#reindex) does, says so on stderr before it starts (`building the search index: <n> entries`, or `rebuilding the search index: <why>`) and can take minutes on a large store. The line comes when the build names more entries than the run has kept, so a first run on a new store, where the entries it kept are all there is to build, says nothing, and neither do two other long builds: a store whose index was lost and whose every entry is captured again in the same run, and a first run that keeps one very large entry. A file of the index that is cut short, or that the index library panics on when it opens it, makes the index broken (the library's own panic text can appear on stderr while the run looks at it), and the run makes it again by itself. A run that finds nothing new does not even load the tokenizer. A run with a target that claims nothing does not open the store, so it does not touch the index.

If the index cannot be read or written, or its folder is not bilbo's, the run prints `the search index was not updated: <error>` and exits 1, and when the index itself failed (not a folder that is not bilbo's, which nothing repairs) the line ends with `; bilbo reindex makes the index again`; damage that shows only when the index's columns are read, which the check of its files does not see, fails the update with the same line (`its content could not be read`, after tantivy's own panic text on stderr), and `bilbo reindex` repairs it; what it kept stays kept, and the run tries the index no more. Nothing is remembered between runs: the next run plans again, and a rebuild that fails late costs its whole time again at every run, up to about five minutes of the store held every ten on a large one, until what makes it fail is fixed. An entry the reader or the tokenizer fails on is named on stderr and exits 1; it is tried again by the next two runs, and then waits for a change to it or for `bilbo reindex`.

An index kept current by `remember` holds the same documents as one `reindex` builds and the same documents match a query, but it can rank them differently until `bilbo reindex` runs.

### Sealing

New records go to `open.seg` in the state folder. A run seals `open.seg` into a segment of the store when it ends, if `open.seg` holds frames and has reached `store.seal_at` or its oldest frame is `store.seal_after` old (60 minutes by default); during a run, `store.seal_at` alone seals. `open.since` in the state folder records when `open.seg` got its oldest frame. A bare run opens the store and seals what is due even when no follow claims a file, so the timer's pass seals by age and rebuilds a lost catalog; a run whose target claims nothing does not open the store. See [Configuration](configuration.md#keys).

### Two runs at once

One bilbo writes the store at a time: the writer holds `store.lock` and `store.writer` in the state folder, and a reader holds `store.lock` shared. A run with no target that finds another bilbo writing sets the again flag (`store.again`), prints `another bilbo is writing the store; requested another pass over the follows` and exits 0 at once; one that finds only readers waits for them, since a reader never runs the follows again. A `remember` or a `reindex` that reaches the end of its work takes the request: before it releases the store, it runs every follow once more for each request it finds, and brings the index up to what each pass kept. A run with a target waits for the store instead, and says so on stderr, since a rerun of the follows would not keep its target.

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

The timer is what keeps a session as it grows: a turn is kept and indexed by the next run after it is written, at most 10 minutes later while runs are short. The 10 minutes are the interval between the timer's starts, not a bound: a run that rebuilds the index takes minutes, and a failing or suspended run takes longer. Without a timer, nothing is kept until `bilbo remember` is run by hand.

### Output

Standard output, two lines:

```text
new <n>, appended <n>, new generation <n>, unchanged <n>, refused <n>
records <n>, raw bytes <n>, stored bytes <n>
```

Raw bytes are the input bytes kept; stored bytes are what the store grew by. A run with no target adds up every follow. On a terminal the same counts come as a marked message. A run that also ran the follows again for another bilbo says what that pass kept in one line on stderr. The index adds lines on stderr only when it rebuilds, leaves an entry out or fails; stdout does not change. A bare run that finds another bilbo writing the store prints nothing on stdout; its planning lines, if any, come before the busy line on stderr.

A second `remember` with a target waits for the first one to finish writing the store, and says so on stderr.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when another bilbo holds the store and a run with no target asked it to run again |
| 1 | a file was refused, nothing was claimed under a target, a follow cannot run (also in a pass run again for another bilbo), the search index could not be updated or an entry could not be indexed, or the target or the store could not be read |
| 2 | usage or config error, a store root that holds bilbo 0.20's `notes/` or `library/`, a state or cache folder inside the root, or a config file `--follow` cannot write |

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

`read` uses the catalog in the cache folder when it holds every frame of the store. When the catalog is missing or behind (the cache folder was deleted, say), `read` walks every segment instead, under the lock, which takes as long as the store is large: reads are slow until the next `bilbo remember` rebuilds the catalog. A `bilbo remember` that finds a reader on the store waits for it.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when no record of the range renders |
| 1 | no entry has the id or the session id, the range or the generation is outside the entry, the entry is of a kind this bilbo cannot read, there is no store, the store is being written for longer than `store.wait`, or the store could not be read |
| 2 | usage or config error: a ref that does not parse, a session id that names several entries, a `--budget` under 256 |

## reindex

```text
bilbo reindex
```

Makes the catalog and the search index again from the store. Both live in the cache folder ([Configuration](configuration.md#folders)) and hold nothing the store does not, so `reindex` loses nothing: run it after the cache folder was deleted or restored from a copy, after `search.chunk_tokens`, `search.chunk_overlap` or `markdown.heading_width` changed, or when an index is in doubt.

### What it makes

- **The catalog**, `catalog.sqlite`: rebuilt from every segment and `open.seg`, as a `bilbo remember` rebuilds a missing one.
- **The index**, the folder `index/`: one document for each chunk of each entry's newest generation. A chunk is consecutive records packed up to `search.chunk_tokens` tokens of the Qwen3-Embedding tokenizer; a record longer than that is cut into chunks that share `search.chunk_overlap` tokens. A record that renders to more than 1 MiB is cut at line ends before it is counted. The index holds each chunk's words (lowercased, without 33 English stopwords, stemmed) and where its records are in the store (entry, generation, record range, date, kind), never its text. A word of more than 4,096 bytes is not indexed.

An entry whose kind no reader reads is left out and counted as unread. Older generations of an entry are not indexed; `bilbo read` still prints them.

`reindex` replaces only its own files: `catalog.sqlite` with its journals, and the files of an index it made. The `index/` folder holds a file `bilbo-index` that marks it as bilbo's. An `index/` folder that holds anything and lacks that file, that holds a stranger beside it, or that is a link is refused, exit 1, and nothing in it is touched: move it away or set `paths.cache` to another folder. The old index stays in place until the new one is whole, so while `reindex` runs the folder holds both.

### What it keeps

The catalog remembers the size and time of each file `remember` kept, so a later run skips a file that has neither changed. `reindex` carries those over from the old catalog. When the old catalog is missing, unreadable or another store's, they are lost, and the first `bilbo remember` after it reads every followed file once more to find that nothing changed.

### While another bilbo runs

`reindex` holds the store's lock from start to end, as a `bilbo remember` does, and waits for one that is writing, saying so on stderr. A `bilbo remember` with a target waits for `reindex`. One with no target that finds `reindex` running exits 0 and asks for one more run, as it asks any writer ([Two runs at once](#two-runs-at-once)). `reindex` closes as `remember` does: once it has committed the index it serves that request by running the follows, grows the index from the cursor it committed after each pass, and seals when a seal is due. Its lines for a pass are `remember`'s, on stderr; stdout counts only its own rebuild; the exit code is 1 also when a pass failed or refused a file. A `reindex` whose own rebuild fails releases the store as it is: it runs no close loop, does not seal and leaves another bilbo's request for the next writer. Only a failure inside its close loop follows `remember`'s rule that the run tries the index no more.

### When it is needed

`remember` keeps the index current, so `reindex` is for an index in doubt, and after a store is restored from a copy, always: bilbo cannot tell an older or branched history that kept its catalog id from a current one. It is also the way to the rebuilt ranking: an index kept by `remember` holds the same documents and the same documents match, but it can rank them differently. A failed entry is tried again by the next two runs of `remember`, then waits for a change or a `reindex`.

### Output

Standard output, two lines:

```text
entries <n>, chunks <n>, unread <n>, failed <n>
index bytes <n>, milliseconds <n>
```

`entries` are the entries indexed, `unread` the ones left out for their kind, `failed` the ones left out whole because their records could not be read back, a reader or the tokenizer failed on them, or the tokenizer could not split a text of them; each kind left out and each failed entry gets a line on stderr, and any failed entry makes the exit code 1. `index bytes` is the size of the index's files, `milliseconds` how long the run took. On a terminal the same counts come as a marked message.

A store that cannot be read, or a damaged frame found while the catalog is rebuilt, stops `reindex` with the store's error and exit 1 before the index is touched.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success, also when entries of a kind no reader reads were left out |
| 1 | an entry could not be read back, the `index/` folder is not bilbo's, or the store or the index could not be read or written |
| 2 | usage or config error, a store root that holds bilbo 0.20's `notes/` or `library/`, or a state or cache folder inside the root |
