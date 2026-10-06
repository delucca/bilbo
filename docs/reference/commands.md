# Commands

Every `bilbo` verb, in the order `bilbo --help` prints them: its synopsis, what
it does and how it exits. Verbs with a guide give a one-line summary and link to
it.

## Exit codes and streams

Every verb except `digest` exits 0 on success, 1 when it refuses the request,
finds problems or, for `recall`, finds nothing, and 2 on a usage error: an
unknown verb or option, a missing or extra argument, or an invalid argument
value. `digest` always exits 0, because a prompt hook that exits 2 blocks the
prompt.

stdout carries only a verb's result. Every diagnostic goes to stderr as a line
starting with `bilbo: `. The wizard of `setup` and the recovery phrase prompts
of `device init` and `device recover` draw on stderr without that prefix.

`bilbo --help` and `bilbo -h` print the usage to stdout and exit 0. `bilbo
--version` prints `bilbo <version>` and exits 0.

## new

```sh
bilbo new <kind> <topic> [--title <text>] [--scope <name>]
```

Creates `<root>/notes/<kind>-<topic>.md`, writes its frontmatter and title, and
prints its path. The kinds, the topic grammar and the frontmatter are in [Files
and formats](files.md#note-format). Exits 1 when the topic already has a note,
and 2 when `--scope` names a scope the config does not declare.

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

It refuses with `bilbo: no sources match` when nothing matches and with `bilbo:
no library at <root>` when the store has no corpus. Plain `recall`, `index` and
the digest never read the library.

## index

```sh
bilbo index
```

Embeds the passages the vector cache lacks and drops the ones no note holds any
more. A note that [the embedder rule](../guides/embedders.md#the-embedder-rule)
withholds is not sent to a remote embedder.

## setup

```sh
bilbo setup [--yes | --interactive] [--remove] [<setup option>]...
```

Creates the store and the config and installs the agent plugin, the index timer,
the note watcher and, when asked, the local embedder. In a terminal it asks
first. Exits 1 when any step failed. The steps, the wizard and every option are
in [Set up bilbo](../guides/setup.md).

## digest

```sh
bilbo digest
```

Run by the plugin's prompt hook. It reads the hook's JSON on stdin and prints
the notes that bear on the prompt. It always exits 0. See [The
digest](../guides/agents.md#the-digest).

## library

```sh
bilbo library [<corpus>]
bilbo library show <corpus>/<name>|<id>[#<anchor>] [--depth <n>]
bilbo library stage <url> | <file> --origin "<url|doc>: <value>" [--fetched <YYYY-MM-DD>] [--html]
bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]
bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>] [--slice-lines <n>]
bilbo library read <plan> <slice>... [--part <k>/<n>]
```

Lists the corpora, prints a corpus's guide or a source's outline, adds a source
with `stage` and `land`, and reads sources through a plan. See [The
library](../guides/library.md).

## cite

```sh
bilbo cite [--plan <plan>]... [<file> | -]
```

Checks every `bilbo:` citation in a draft from a file or stdin, and with
`--plan` prints the coverage of the plans' reads. It reads no settings and
writes nothing. Exits 1 when a citation has a failing verdict. See
[Citations](../guides/library.md#citations).

## watch

```sh
bilbo watch
```

Records a version of each note when it changes, until it is stopped, and syncs
the scopes that sync. See [Note history](../guides/history.md).

## history

```sh
bilbo history <note> [<version> | --diff <a> [<b>]]
```

Lists a note's versions, newest first, prints one, or shows what changed between
two versions, or between one and the note's file now. It changes nothing. See
[Note history](../guides/history.md#read-a-notes-history).

## restore

```sh
bilbo restore <note> <version>
```

Writes a past version of a note back as its newest version, keeping what the
note held before. See [Note history](../guides/history.md#restore-a-version).

## scope

```sh
bilbo scope
bilbo scope set [--force] <name> <file>...
```

`scope` lists the scopes this device declares with their note counts. `scope
set` gives notes a scope. See [Scopes](../guides/scopes.md).

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
prints a problem line for a scope. See [Devices](../guides/devices.md).

## sync

```sh
bilbo sync
bilbo sync declare <note> <reason>
```

`sync` prints each syncing scope's state, its devices, the open conflicts and
the dropped text nobody declared, and exits 1 when something needs you. `sync
declare` records that text a resolution dropped was dropped on purpose. See
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
bilbo relay --data <dir> --owner <fingerprint>... [--listen <address:port>] [--max-scopes <n>] [--max-scope-mb <n>] [--max-object-mb <n>]
```

Serves sync to your devices from a machine you control, until it is stopped. See
[Run a relay](../guides/relay.md).

## See also

- [Configuration](configuration.md) for the config keys and environment
  variables.
- [Files and formats](files.md) for the note and library formats.
