# Commands

Every `bilbo` verb: its synopsis, what it does and how it exits. Verbs with a
guide give a one-line summary and link to it. `bilbo <verb> --help` prints the
same synopsis with every option and its default, the output and the exit codes.

## Exit codes and streams

Every verb except `digest` exits 0 on success, 1 when it refuses the request,
finds problems or, for `recall`, finds nothing, and 2 on a usage error: an
unknown verb or option, a missing or extra argument, or an invalid argument
value. `digest` always exits 0, because a prompt hook that exits 2 blocks the
prompt.

stdout carries only a verb's result. Every diagnostic goes to stderr. Off a
terminal, or when an agent runs bilbo, each stderr line starts with `bilbo: `.
On a person's terminal it starts with a mark instead: `■` error, `▲` warning,
`●` progress, `○` nothing found, and warnings come after the result. The wizard
of `setup` and the recovery phrase prompts of `device init` and `device recover`
draw on stderr with neither. See [Terminal output](#terminal-output).

`bilbo --help`, `bilbo -h` and `bilbo help` print an overview of the verbs to
stdout and exit 0. `bilbo <verb> --help`, `bilbo <verb> -h` and `bilbo help
<verb>` print that verb's page: its synopsis, options, output, exit codes and
examples. A usage error prints its reason, then the synopsis of the verb that
was run and the page to read, to stderr, and exits 2. `bilbo --version` prints
`bilbo <version>` and exits 0.

## Terminal output

Every verb that shows a result has two views of it. The plain view is the bytes
shown on this page: tab-separated lines, `bilbo: ` on stderr. It is what pipes,
files, hooks and agents get, and it is the contract that scripts and the plugin
read. The terminal view is the same facts laid out for a person: marks, aligned
tables, grouped lists, ages and a little colour. bilbo prints it only when the
stream is a terminal and no agent marker is set. The `console` blocks on this
page, and on the guides, show the plain view unless they say otherwise.

The gate is checked per stream, so `bilbo recall x | less` prints the plain
view to the pipe and the terminal form of stderr to the screen. The agent
markers are `AI_AGENT`, `CLAUDE_CODE_CHILD_SESSION`, `CODEX_THREAD_ID` and
`CODEX_CI`: when any is set and not empty, bilbo prints the plain view even on
a terminal. `CLAUDECODE` is not one, so a person's own terminal inside an IDE
that sets it still gets the terminal view. To see the plain view in a terminal,
run `bilbo <verb> | cat`.

The terminal view is tuned by the environment:

| Variable | Effect |
|---|---|
| `NO_COLOR` (not empty), `CLICOLOR=0`, `TERM` unset or `dumb` | No escapes; the layout stays. |
| `CLICOLOR_FORCE`, `FORCE_COLOR` | Ignored: forcing colour into a pipe would break it. |
| `COLUMNS` | Used only when the terminal reports no size; 80 when neither is known. The width is kept between 40 and 100 columns. |
| `LANG` | Outside macOS, the marks fall back to ASCII unless it names UTF-8. |

The palette is small: bold for titles, a table's first column and matched
query words; dim for paths, ids, times and separators; cyan for words you type
back, such as kinds, scopes and versions; green for done, yellow for a warning,
red for an error. The marks are `◆` done or changed, `◇` kept or unchanged, `○`
nothing found, `▲` warning, `■` error and `●` progress; in ASCII they are `*`,
`o`, `-`, `!`, `x` and `•`. A heading path reads `A › B`, where the plain view
has `A > B`. Paths show `~` for the home folder, times read `17 min ago`,
`yesterday` or `3 days ago` up to 30 days and then the date, and numbers group
by thousands.

These verbs print the plain view everywhere, because a script or an agent reads
them: `digest`, `cite`, `library plan`, `library read`, `library stage`,
`library land`, `scope set`, `sync declare`, `pair`, `relay` and `watch`. A
printed note version (`history <note> <version>`) is the file's own bytes.

## new

```sh
bilbo new <kind> <topic> [--title <text>] [--scope <name>]
```

Creates `<root>/notes/<kind>-<topic>.md`, writes its frontmatter and title, and
prints its path. The kinds, the topic grammar and the frontmatter are in [Files
and formats](files.md#note-format). Exits 1 when the topic already has a note,
and 2 when `--scope` names a scope the config does not declare. On a terminal
it prints `◆  Created <kind> <topic>`, the path on the next line, and then any
warning.

`new` picks the scope from `--scope`, else `paths`, else `scope.default`; see
[Scopes](../guides/scopes.md#how-new-picks-a-scope).

## check

```sh
bilbo check
```

Lints the whole store, the notes and the library, against their rules, and
prints every problem, one per line. It changes nothing, so mistakes an agent
makes while editing files by hand surface without bilbo blocking anything. Exits
1 when it finds any problem.

```console
$ bilbo check
notes/plan-broken.md: created: missing
notes/plan-broken.md: title: missing; add one '# <title>' line after the frontmatter
```

On a terminal, problems are grouped under each file with a summary, and a
clean store prints `◆  No problems in <n> notes and <c> corpora`.

With scopes declared, `check` also reports notes with no scope or an undeclared
one, and marks; see
[Scopes](../guides/scopes.md#check-the-scopes-of-your-notes).

## recall

```sh
bilbo recall <query>... [--kind <kind>]... [--limit <n>]
bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]
```

Prints the notes that best match, best first, 10 unless `--limit` says
otherwise. `--kind` narrows the search to a kind and repeats. Exits 1 when
nothing matches.

`recall` prints one block per note: the path and line of the best passage, the
kind and `created` (tab-separated), then the passage's heading path, then the
first 300 characters of its text:

```console
$ bilbo recall busy timeout
/Users/me/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:11	gotcha	2026-10-06T14:39-03:00
SQLite needs a busy timeout > Fix
Set `busy_timeout = 5000` right after opening the connection.
```

That is the plain view, which is what a pipe, a hook and an agent get. On a
terminal, `recall` ranks the hits, shows each note's title and headings, a
cleaned snippet with the query words in bold that starts near the first match,
and a dim line with the kind, age and path, then a count line:

```console
$ bilbo recall busy timeout
 1  Sqlite busy timeout > Fix
    Set busy_timeout = 5000 right after opening the connection, before any query. Without it a
    second writer gets SQLITE_BUSY at once instead of waiting.
    gotcha · 17 min ago · ~/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:9

1 note, best first
```

When nothing matches, a terminal gets `○  no notes match '<query>'` and a hint
on the next line; the plain form is the `bilbo: no notes match` line on stderr.

Without an embedder, `recall` matches whole words, ignoring case and accents.
With one, it fuses that order with a ranking by meaning. When the embedder is
down it falls back to keywords and says so on stderr; passages written since the
last `bilbo index` rank by keywords only, and `recall` says that too. See
[Search by meaning with an embedder](../guides/embedders.md).

`recall --library` searches the sources and guides of the library by keyword
instead of the notes; `--corpus` narrows it to a corpus and repeats. A block
holds the path and line of the best passage, `source` or `guide`, the reference
`bilbo library` takes to show the file (`<corpus>/<name>` for a source,
`<corpus>` for a guide) and the lines of the passage's section (tab-separated),
then the heading path below the title (`-` for the title passage), then the
snippet:

```console
$ bilbo recall --library --corpus go goroutine leak
/Users/me/.local/share/bilbo/library/go/effective-go.md:340	source	go/effective-go	340-380
Concurrency > Goroutines
They're called *goroutines* because the existing terms ...
```

`recall --library` has the same two views. It refuses with `bilbo: no sources
match` (on a terminal `○  no sources match '<query>'`) when nothing matches and with
`bilbo: no library at <root>` when the store has no corpus. Plain `recall`, `index` and
the digest never read the library.

## index

```sh
bilbo index
```

Embeds the passages the vector cache lacks and drops the ones no note holds any
more. A note that [the embedder rule](../guides/embedders.md#the-embedder-rule)
withholds is not sent to a remote embedder. On a terminal it prints `◆  Embedded
<n> passages · kept <k> · dropped <d>`; plain, it prints the same counts as
today.

## setup

```sh
bilbo setup [--yes | --interactive] [--remove] [<option>]...
```

Creates the store and the config and installs the agent plugin, the index timer,
the note watcher and, when asked, the local embedder. In a terminal it asks
first. Exits 1 when any step failed. The steps, the wizard and every option are
in [Set up bilbo](../guides/setup.md). The report is a marked step list with a
summary on a terminal.

## digest

```sh
bilbo digest
```

Run by the plugin's prompt hook. It reads the hook's JSON on stdin and prints
the notes that bear on the prompt. It always exits 0. Plain on a terminal too.
See [The digest](../guides/agents.md#the-digest).

## library

```sh
bilbo library [<corpus>]
bilbo library show <ref> [--depth <n>]
bilbo library stage <url>
bilbo library stage <file> --origin "<url|doc>: <value>" [--fetched <YYYY-MM-DD>] [--html]
bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]
bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>] [--slice-lines <n>]
bilbo library read <plan> <slice>... [--part <k>/<n>]
```

`<ref>` is `<corpus>/<name>` or a source id, then optionally `#<anchor>`.

Lists the corpora, prints a corpus's guide or a source's outline, adds a source
with `stage` and `land`, and reads sources through a plan. See [The
library](../guides/library.md). On a terminal the corpora are a table with a
header, a guide shows the lead and each entry with its facts, and `show` prints
a header, the facts and an outline table; `stage`, `land`, `plan` and `read`
stay plain.

## cite

```sh
bilbo cite [--plan <plan>]... [<file> | -]
```

Checks every `bilbo:` citation in a draft from a file or stdin, and with
`--plan` prints the coverage of the plans' reads. It reads no settings and
writes nothing. Exits 1 when a citation has a failing verdict. Plain on a
terminal too. See
[Citations](../guides/library.md#citations).

## watch

```sh
bilbo watch
```

Records a version of each note when it changes, until it is stopped, and syncs
the scopes that sync. See [Note history](../guides/history.md).

## history

```sh
bilbo history <note>
bilbo history <note> <version>
bilbo history <note> --diff <a> [<b>]
```

Lists a note's versions, newest first, prints one, or shows what changed between
two versions, or between one and the note's file now. It changes nothing. On a
terminal the list is a table with ages and a diff is coloured; equal versions
print `◇  no changes between <a> and <b>`. See
[Note history](../guides/history.md#read-a-notes-history).

## restore

```sh
bilbo restore <note> <version>
```

Writes a past version of a note back as its newest version, keeping what the
note held before. On a terminal it prints `◆  Restored <file> to <version>`. See
[Note history](../guides/history.md#restore-a-version).

## scope

```sh
bilbo scope
bilbo scope set [--force] <name> <file>...
```

`scope` lists the scopes this device declares with their note counts, as a table
with a header on a terminal. `scope set` gives notes a scope. See [Scopes](../guides/scopes.md).

## device

```sh
bilbo device
bilbo device list
bilbo device init [--name <name>]
bilbo device recover [--name <name>]
bilbo device revoke <device>
```

Shows and manages this device's identity: the recovery phrase, the owner key,
this device's key and each syncing scope's manifest. `device` exits 1 when it
prints a problem line for a scope. On a terminal `device` and `device list` are
a grouped status and a table, and `device init` shows its steps with marks. See
[Devices](../guides/devices.md).

## sync

```sh
bilbo sync
bilbo sync declare <note> <reason>
```

`sync` prints each syncing scope's state, its devices, the open conflicts and
the dropped text nobody declared, and exits 1 when something needs you. `sync
declare` records that text a resolution dropped was dropped on purpose. On a
terminal the report is grouped by scope, with ages and a mark for each device;
`sync declare` stays plain. See
[Sync](../guides/sync.md).

## pair

```sh
bilbo pair [--scope <name>]... [--via <url>]
bilbo pair <code> --via <url> [--name <name>]
```

The first form shows a one-time code on an enrolled device and adds the device
that answers it to your scopes. The second joins the scopes of the device that
showed the code. See [Pair a device](../guides/devices.md#pair-a-device).

## relay

```sh
bilbo relay --data <dir> --owner <fingerprint> [--owner <fingerprint>]... [--listen <address:port>] [--max-scopes <n>] [--max-scope-mb <n>] [--max-object-mb <n>]
```

Serves sync to your devices from a machine you control, until it is stopped. See
[Run a relay](../guides/relay.md).

## See also

- [Configuration](configuration.md) for the config keys and environment
  variables.
- [Files and formats](files.md) for the note and library formats.
