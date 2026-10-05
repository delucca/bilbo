# bilbo

Durable memory for coding agents: Markdown notes an agent writes in one
session and finds again in the next.

Agents re-derive what an earlier session already worked out. bilbo gives them
a store of plain Markdown notes they write with their own file tools, and a
`bilbo` command that starts, checks, searches and indexes those notes. A
`note` skill and a `recall` skill for Claude Code and Codex put the writing and
the search in the agent's hands, and a `reference` skill answers from the
library's sources, read in full and cited by id. An `ingest` skill adds those
sources: a page, a file or a PDF's text.

- **Plain files.** One note per topic in `<root>/notes/`, named
  `<kind>-<topic>.md`, with a small YAML frontmatter. Read, edit, grep or
  version them like any other file.
- **Keyword search out of the box, meaning search with an embedder.** Point
  bilbo at Ollama, OpenAI or any OpenAI-compatible URL, or let it run a local
  model, and `recall` finds notes that share no word with the query.
- **History for every note.** A background watcher records each version of a
  note as an agent or an editor changes it, so a careless rewrite is one
  `bilbo restore` away from undone.
- **Agent plugin.** `bilbo setup` installs the plugin in Claude Code and Codex
  and a timer that keeps the index current. The plugin also hands the agent the
  notes that bear on each prompt, before it starts, and reminds it to write
  down what a session settled once its context is compacted. Its `ingest`
  skill stages a URL, cuts the capture to the document and lands it as a
  source. Its `reference` skill reads sources through `bilbo library plan` and
  `read`, and checks every citation with `bilbo cite`, so the coverage it
  reports is counted by bilbo, not by the agent.

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
`doc` or `search`. `scope` is optional too, a name from [Scopes](#scopes). No
other key is allowed. The store root is `$BILBO_HOME`,
else `$XDG_DATA_HOME/bilbo`, else `~/.local/share/bilbo`, on macOS too.

### Commands

| Command | What it does |
| --- | --- |
| `bilbo new <kind> <topic> [--title <text>] [--scope <name>]` | Creates the note and prints its path. Exits 1 when the topic already has a note. See [Scopes](#scopes). |
| `bilbo check` | Prints every problem in the store, one per line, and changes nothing. Exits 1 when it finds any. |
| `bilbo recall <query>... [--kind <kind>]... [--limit <n>]` | Prints the notes that best match, best first, 10 by default. Exits 1 when nothing matches. |
| `bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]` | Prints the sources and guides of the library that best match, by keyword, one block per file. Exits 1 when nothing matches. |
| `bilbo scope` | Lists the declared scopes and the notes in each; see [Scopes](#scopes). |
| `bilbo scope set [--force] <name> <file>...` | Gives each note the scope; see [Scopes](#scopes). |
| `bilbo index` | Embeds the passages the vector cache lacks and drops the ones no note holds any more. |
| `bilbo digest` | Run by the plugin's prompt hook; see [The digest](#the-digest). |
| `bilbo library` | Lists and shows the library, and stages and lands its sources; see [Library](#library). |
| `bilbo cite [--plan <plan>]... [<file> \| -]` | Checks every `bilbo:` citation in a draft; see [Citations](#citations). |
| `bilbo watch` | Records each change to a note; see [History](#history). |
| `bilbo history <note> [<version> \| --diff <version> [<version>]]` | Lists a note's versions, prints one, or diffs two; see [History](#history). |
| `bilbo restore <note> <version>` | Writes a past version back as the note's newest; see [History](#history). |
| `bilbo sync` | Prints how each syncing scope is doing and what waits; see [Sync](#sync). Exits 1 when something needs you. |
| `bilbo sync declare <note> <reason>` | Records that text a resolution dropped was dropped on purpose; see [Sync](#sync). |
| `bilbo device [list \| init \| recover \| revoke <device>]` | Shows and manages this device's identity; see [Devices](#devices). |
| `bilbo pair [--scope <name>]... [--via <url>]` | Shows a one-time code and adds the device that answers it to your scopes; see [Pairing a device](#pairing-a-device). |
| `bilbo pair <code> --via <url> [--name <name>]` | Joins the scopes of the device that showed the code; see [Pairing a device](#pairing-a-device). |
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

`recall --library` searches the sources and guides of the library by keyword
instead of the notes; `--corpus` narrows it to a corpus and repeats. A block
holds the path and line of the best passage, `source` or `guide`, the
reference `bilbo library` takes to show the file (`<corpus>/<name>` for a
source, `<corpus>` for a guide) and the lines of the passage's section
(tab-separated), then the heading path below the title (`-` for the title
passage), then the snippet:

```console
$ bilbo recall --library --corpus go goroutine leak
/Users/me/.local/share/bilbo/library/go/effective-go.md:340	source	go/effective-go	340-380
Concurrency > Goroutines
They're called *goroutines* because the existing terms ...
```

It refuses with `bilbo: no sources match` when nothing matches and with
`bilbo: no library at <root>` when the store has no corpus. Plain `recall`,
`index` and the digest never read the library.

`check` lints the whole store, the notes and the library, against their rules, so
mistakes an agent makes while editing files by hand surface without bilbo
blocking anything:

```console
$ bilbo check
notes/plan-broken.md: created: missing
notes/plan-broken.md: title: missing; add one '# <title>' line after the frontmatter
```

`bilbo --help` prints the full usage.

### Scopes

A scope is a name for a part of your work, such as `work` or `personal`. A note
says which one it belongs to with a `scope: <name>` line in its frontmatter.
The name follows the topic's grammar: lowercase words joined by single hyphens.
What a scope means is set per device in the [config](#configuration), so the
same note can sit in `work` on one machine and be unassigned on another.

A scope exists on a device when its config holds at least one `scope.<name>.*`
key. A note is unassigned when it has no `scope` line, or names a scope the
config does not declare. A scope has four settings:

```
scope.personal.sync = off
scope.work.embedder = local
scope.work.paths = ~/Developer/acme
scope.work.marks = acme, ~/Developer/acme
scope.default = personal
```

- `sync` is `off`, the default, or a URL; see [Sync URLs](#sync-urls). A URL
  only records where the scope will sync; `bilbo device init` makes its
  manifest.
- `embedder` is `any` (default) or `local`. A `local` scope's notes are never
  sent to an embedder outside this machine; see [The embedder
  rule](#the-embedder-rule).
- `paths` lists folders, comma-separated. `bilbo new` gives a note the scope
  whose entry is the working directory or a folder above it, the longest entry
  winning, compared by whole folder names. `~/` and `/` are valid entries, so
  `scope.personal.paths = ~/` makes `personal` the scope of everything under
  home that no longer entry claims. A worktree outside every listed folder is
  not covered: list the folder that holds the clones and `.worktrees/`, not
  each clone.
- `marks` lists what only this scope's notes should mention, for `check`.
- `scope.default` names the scope for a working directory no `paths` entry
  holds.

`bilbo new` picks the scope from `--scope`, else `paths`, else `scope.default`.
`--scope` must name a declared scope, or it exits 2. When nothing matches and
any scope is declared, the note is created without a `scope` line and stderr
says so:

```console
$ bilbo new plan release
/Users/me/.local/share/bilbo/notes/plan-release.md
bilbo: no scope for /Users/me/.local/share/bilbo/notes/plan-release.md; scopes: personal, work; set one with bilbo scope set <name> /Users/me/.local/share/bilbo/notes/plan-release.md
```

A mark is either one word (letters and digits only, so `acme.`, `Acme's` and
`ac-me` are config errors) or a path starting with `/` or `~/`. A word matches
a whole word of the note's file name topic, `sources` or body, fenced code
included, ignoring case and accents. A path matches where the next character
ends a path segment: not a letter, a digit, `-`, `_` or `.`. A path inside home
matches as `~/...` and as the absolute path, so `~/Developer/acme` finds
`/Users/me/Developer/acme/api/main.go` but not `~/Developer/acme-tools`. List
words first: the employer, its products and its repo names. Path marks catch
the absolute paths that end up in bodies and code blocks. Two scopes cannot
share a path or a mark.

`bilbo check` reads the config and reports a note that has no `scope` line
(once any scope is declared) or names a scope the config does not declare, and
lists the declared scopes:

```console
$ bilbo check
notes/gotcha-acme-deploy.md: scope: 'acme' is not declared in /Users/me/.config/bilbo/config; scopes: personal, work
notes/gotcha-deploy.md: scope: 'personal' but line 9 holds 'acme', a mark of 'work' (warning)
notes/plan-release.md: scope: missing; scopes: personal, work
notes/plan-x.md: scope: missing; scopes: personal, work; holds marks of work
```

An unassigned note that holds a mark gets `; holds marks of <scopes>` on its
problem, to say where to put it. A note in a scope that holds another scope's
mark gets one `(warning)` line per other scope: the first place is `the file
name` or `line <n>`. A warning leaves the exit code alone, and a mark never
holds anything back. A mark of `~/` warns on every path under home.

`bilbo scope` prints one line per declared scope, sorted by name, with tab-separated
fields: the name, the note count, `sync`, `embedder`, `paths` as written (`-`
when unset) and `default` for the default scope. The last line is the
unassigned notes:

```console
$ bilbo scope
personal	3 notes	sync off	embedder any	paths -	default
work	1 notes	sync off	embedder local	paths ~/Developer/acme
(unassigned)	2 notes	embedder local
```

With no scope declared it prints only the `(unassigned)` line and says on
stderr, `bilbo: no scopes declared; add scope.<name>.* keys to <config path>`.

`bilbo scope set [--force] <name> <file>...` gives each note the scope. A note
with no `scope` line gains one as the last line of its frontmatter and prints
`notes/<file>: set <name>`. A note already in `<name>` prints `kept <name>`. A
note in another scope prints `kept <old>; --force replaces it`, and with
`--force` `replaced <old> with <name>`. A key line not written as
`scope: <value>`, such as `scope:work`, prints `kept 'scope:work' as written;
--force rewrites it`, and with `--force` `rewrote 'scope:work' as scope: work`.
No other byte of the file changes. A file that is not a note directly in
`<root>/notes/`, whose text is not UTF-8 or whose frontmatter is broken, is refused on stderr and the run exits 1 after handling every file; an
undeclared name or missing files exit 2. It works under the history lock and
swaps the file in atomically, so a write by an agent in the same instant is
never lost: the run reports `changed while bilbo scope set ran; run it again`
and leaves the agent's text. A filesystem that cannot swap files atomically is
refused.

To triage the notes a store already holds, run two passes, the specific glob
first. Notes that already have a scope are kept:

```sh
bilbo scope set work ~/.local/share/bilbo/notes/*acme*.md
bilbo scope set personal ~/.local/share/bilbo/notes/*.md
bilbo check
```

### Devices

A device is one machine's copy of bilbo, and an owner is you: one identity
that every device of yours shares. `bilbo device` creates and shows these
identities. They are what [Sync](#sync) signs and encrypts with.

| Command | What it does |
| --- | --- |
| `bilbo device` | Prints this device, the owner fingerprint and one line per syncing scope. Exits 1 when it prints a problem line for a scope. |
| `bilbo device list` | Lists the enrolled devices: name, id, and `this` for this one. |
| `bilbo device init [--name <name>]` | Creates the recovery phrase, the owner key and this device's key, then a manifest for each scope with a sync URL. Run again, it brings the manifests in line with the config. |
| `bilbo device recover [--name <name>]` | Rebuilds the owner key from the typed phrase on a new or wiped device and adds the device to the scopes the store holds. |
| `bilbo device revoke <device>` | Removes a device, by name or id, from every scope that lists both it and this device, and starts a new epoch key. |

The device name is the host name up to its first `.`, lowercased, unless
`--name` gives one: lowercase words joined by single hyphens, at most 32
characters. A device's id is derived from its key.

#### Sync URLs

`scope.<name>.sync` is `off`, the default, or a URL that says where the scope
will sync:

```
scope.personal.sync = file:///Users/me/Library/Mobile Documents/bilbo
scope.work.sync = https://relay.example.net:8443/bilbo
scope.dev.sync = http://127.0.0.1:8740
```

- `file://` and an absolute path, taken literally: a space stays a space, and
  `%20` is a folder named with `%20`. Put the folder a cloud service syncs here.
- `https://<host>[:<port>][/<prefix>]`, or `http://` only to `localhost`,
  `127.0.0.1` or `::1`.
- No user name, password, query or fragment. A bad value is a config error
  that names the key and exits 2.

#### The ceremony

`bilbo device init` on a device with no keys shows 12 words, the recovery
phrase, and the owner fingerprint, six groups like `yb4b-5aju-v6zb-x2nm-nc5x-ompf`.
Write both down, the fingerprint beside the words. It then asks for 3 of the
words back, and writes nothing until they match. The phrase is the only way to
get back into your scopes when every device is lost: lose it and every device,
and the encrypted scopes are gone. bilbo keeps it in no file and never prints
it to stdout.

The words are drawn on the terminal's alternate screen, and when the ceremony
ends or you cancel, they are gone from the screen and the scrollback. Before
drawing them, bilbo stops the process from writing a core file. A recording of
the terminal, `tmux capture-pane` while the words are up, or a person behind
you still sees them, as they would a sheet of paper.

`recover`, `revoke` and any `init` that creates a phrase or changes a scope's
pinned URL (a folder's path is not pinned) run only in a terminal: stdin and stderr must both be one,
and `CLAUDECODE` and `CODEX_THREAD_ID` must be unset or empty. An agent that
runs them gets one line asking you to run the form yourself. This stops
accidental and low-effort misuse, such as an agent that reaches for a
one-line verb. It does not stop a hostile agent, which can unset the
variables, fake a terminal, or copy the key files. The key files are the real
boundary. Running `bilbo device init` again on an enrolled device needs no
terminal, so an agent can create the manifest of a scope whose URL you added.

#### Where keys live

```
<state>/bilbo/keys/
  owner.key     0600
  device.key    0600
```

`<state>` is `$XDG_STATE_HOME`, else `~/.local/state`. The folder is 0700. The
keys live outside the store, so copying or backing up the store never clones
an identity. `owner.key` holds the owner's signing seed and public key, never
the phrase and never the owner's private box key; `device.key` holds this
device's own keys and name. They are as safe as `~/.ssh/id_ed25519`: any
program that runs as you can read them. A command that reads the keys refuses
when the folder or a file grants anything to group or others, and names the
`chmod` that fixes it. Secrets are zeroed in memory as far as bilbo can; the
terminal emulator, swap and the prompt library's line buffers are not covered.

`bilbo setup --remove` leaves the keys. To forget an identity, delete
`<state>/bilbo/keys/` and `<root>/.bilbo/scopes/` by hand.

#### A second device

`bilbo device init` on a second machine would make a second owner, so it
refuses once the store holds another owner's manifests. For everyday use,
[pair](#pairing-a-device) the machine with one that is enrolled; the phrase
below is for when none is left. When the
scope's `sync` is a `file://` folder, set it in the config and run
`bilbo device recover`: it copies the owner's scope from the folder, as
[Sync](#a-second-device) describes. Otherwise copy the store to the new machine
first, with the sync tool you already use or a plain copy, then run
`bilbo device recover` and type the phrase. recover shows the
fingerprint it derived; when a manifest in the store vouches for the phrase it
asks nothing more, and otherwise it asks you to compare the fingerprint with
the one you wrote down. It then writes this device's keys and adds the device
to every scope of yours the store holds. A syncing scope with no manifest in the copied
store or the folder is reported `unsealed`: bring its manifest over and run
`recover` again, or, when the folder holds none of this owner's `personal`,
run `bilbo device init`.
Never run `init` for it, which would fork the scope.

#### Pairing a device

`bilbo pair` adds a device with a short code typed from one you already have,
so the recovery phrase stays put away. On the enrolled device, in a terminal:

```console
$ bilbo pair
pairing code 42-orbit-tunnel-velvet
on the new device, run: bilbo pair 42-orbit-tunnel-velvet --via file:///Users/me/Dropbox/bilbo
the code works once, for 10 minutes
```

On the new device, install bilbo, run `bilbo setup` so the store exists, and
type the code with the folder as that machine sees it:

```sh
bilbo pair 42-orbit-tunnel-velvet --via file:///home/me/Dropbox/bilbo
```

The code is a number and three words. Case, spaces for hyphens and the first
four letters of a word are all accepted: `"42 ORBI tunn velvet"` is the same
code. `--via` is the folder's path on the new device, which differs from the
path on the first one, and bilbo writes it to the new device's config as the
scope's `sync`. The first device pairs every syncing scope, or only the ones
`--scope <name>` names, up to 12; `--scope` picks which scopes the new device
can read. Scopes on different folders need one `bilbo pair --scope <name>...`
each, naming the scopes of one folder; bilbo refuses and names both URLs when
it cannot tell.

Both devices then print the same fingerprint, twelve digits in three groups of
four, and the new one adds its name and device id. The first device names the
new one and the scopes it will join, and asks:

```console
fingerprint 5812 0934 7761
pair bagend 7f3a9c0e1b2d into personal? compare the fingerprint on that device, then type y to confirm
```

The new device prints `fingerprint 5812 0934 7761 for bagend 7f3a9c0e1b2d;
confirm on the device that showed the code`.

Compare the fingerprint, the name and the id on the two screens, and answer
`y` only when they match. Anything else, or the end of input, sends no secret and both devices exit 1.
The new device then waits up to 2 minutes for the manifests to reach it
through the folder, checks the whole chain, and only then writes its config
and keys. It prints `paired with rivendell: personal`, and `bilbo watch` starts
syncing the scope within one cycle.

A code works for one answer and for 10 minutes. A wrong word uses it up, and
both devices say so; run `bilbo pair` again for a new code. A mailbox nobody
answered is removed when the code expires, and any `bilbo pair` removes one
whose first message is more than 30 minutes old.

Showing a code needs a terminal, as [the ceremony](#the-ceremony) does: stdin
and stderr must be one, and `CLAUDECODE` and `CODEX_THREAD_ID` must be unset or
empty. The confirmation is yours to give, not an agent's. As there, the rule
stops accidental and low-effort misuse and nothing more; the key files' modes
are the last line.

An enrolled device pairs too, to join a scope it is not in: run
`bilbo pair --scope shared` on a device in `shared` and answer on the other
with `bilbo pair <code> --via <url>`. Its keys do not change, and it must
belong to the same owner. A new device that stops after checking the
manifests, for example on a config it cannot write, keeps those manifests but
no keys: pairing it again with the same owner finishes, and pairing it with
another owner is refused until its store is cleared. When no enrolled device is left, the
[phrase](#a-second-device) is the way in, with `bilbo device recover`.

`--scope` decides what the new device can read: a stolen paired device reads
only the scopes it was paired into. What a revoked or stolen device can and
cannot do is what [revoking](#revoking-a-device) says: "After a confirmed
revocation a revoked device, even one using the owner signing seed, cannot read
anything written under later epochs; it can still disrupt by signing versions
that members reject or that change the device list, which watch announces." It
keeps the notes and keys it already held. Whoever can write the shared folder can
stop a pairing, by removing the mailbox or answering first, but cannot read
it: the code never crosses the folder, and the mailbox holds no key, scope
name or URL in the clear. A wrong guess gets one attempt, which uses the code
up and shows on both screens.

#### Revoking a device

`bilbo device revoke bagend`, in a terminal on another device, writes a new
version of each scope that lists both `bagend` and this device, under a new epoch key sealed only to
the remaining devices and to you. After a confirmed revocation a revoked device, even one using the owner signing seed, cannot read anything written under later epochs; it can still disrupt by signing versions that members reject or that change the device list, which watch announces.

That holds for the revoked device's own keys. Until the revocation is confirmed it still holds the current epoch key and can add a device of its own, which the new epoch is then sealed to, so run `bilbo device list` after revoking and revoke any device you do not recognise.

What it does not do: the revoked device keeps every note and key it already
had, and until the revoking version is confirmed writers keep using the old
epoch. A scope that syncs through a `file://` folder also leaves the revoked
device with write access to the folder, so `revoke` prints a line telling you
to remove it from the cloud account that syncs the folder. Holding the owner
seed, it can also create scopes of its own, one of them named like a scope you
have not added yet; check the devices `bilbo device` shows for a new scope.

When a revoked device keeps disrupting, the remedy is a new owner, done by
hand:

1. On a trusted device, move `<state>/bilbo/keys/` and `<root>/.bilbo/scopes/`
   aside.
2. Run `bilbo device init`, which makes a new phrase, a new owner and new scope
   ids.
3. Recover the other devices from the new phrase, and give any relay the new
   fingerprint.

### Sync

`bilbo watch` keeps the notes of a scope the same on all your devices. It
needs a folder that a tool you already use syncs between them, such as
Dropbox, iCloud Drive or Syncthing. bilbo writes into that folder only
encrypted, signed files, and each device writes only its own, so the tool never
sees two writers on one file. The folder is the whole transport: bilbo runs no
server, and an `https://` URL waits for the relay, reported as
`<scheme> transports are not supported yet; use a file:// folder`.

#### Turning sync on

1. Declare the scope's folder in the [config](#configuration):

   ```
   scope.personal.sync = file:///Users/me/Dropbox/bilbo
   ```

2. Create the keys with `bilbo device init`, in a terminal, and write down
   the phrase; see [The ceremony](#the-ceremony). It writes the scope's
   manifest.
3. Make sure `bilbo watch` runs, which `bilbo setup` installs. It syncs the
   scope from then on. The wizard asks for the scope, the folder and the keys
   in one go, and writes the config line.
4. Give the notes the scope. A note syncs only when its `scope:` line names a
   scope whose `sync` is a URL, so turning sync on uploads nothing until you
   assign notes; triage the store as in [Scopes](#scopes).

`bilbo setup` creates the folder's last component when it is missing and its
parent exists. Watch never creates a folder: one that vanishes is reported as
not reachable.

`bilbo sync` then shows how it goes.

#### A second device

Copy nothing. Install bilbo, point the same scope at the same folder once the
cloud tool has synced it, run `bilbo device recover`, type the phrase, and
start `bilbo watch`:

```
scope.personal.sync = file:///Users/me/Dropbox/bilbo
```

`recover` reads the owner's scopes in the folder, finds the one named
`personal` and adds this device to it, so the phrase alone is enough, even when
every other device is lost. Watch then reads every segment of every device
from the first and writes each note once. A device joins a scope only through
`recover` or a version another device wrote, never through watch alone.

#### What syncs

Only the notes in a scope that syncs, and only their text and frontmatter:

- A note with no `scope`, a scope the config does not declare, or a scope whose
  `sync` is `off` never leaves the device, in any form. `bilbo sync` counts
  them: `local: 188 notes sync nowhere`.
- Nothing outside `<root>/notes/` syncs: not the library, the vector cache, the
  config or the keys.
- A deletion syncs. A note deleted on one device while another edits it comes
  back with the edit, flagged `edit-beat-delete`.
- Two notes created offline with one topic keep both: the one with the later id
  is renamed to `<kind>-<topic>-<4 id characters>.md` on every device, flagged
  `topic-taken`.
- Moving a note out of a scope pushes none of its text into the old scope,
  only a marker. The other devices of that scope remove their copy, keep its
  history and print `notes/<file> left the scope; its history stays`. Until 30
  days pass, `bilbo check` warns on the device that moved it.
- A note's text is encrypted under a key only your devices hold. The folder's
  tool and anyone with access to the folder still see the file names (device
  ids and sequence numbers, not notes) and their sizes and times.

A version is pushed when watch records it. Other devices' files are polled
every `sync.poll_seconds`, 30 by default. A device that is offline, or whose
folder is not reachable or full, keeps recording history and retries: watch
says so once on stderr, and `bilbo sync` shows it.

#### Conflicts

Two devices that edit one note concurrently are merged three-way against the
last version they share, passage by passage, a passage being what
[recall](#commands) cuts at a heading. A passage changed on one side takes
that side. Frontmatter merges key by key: `id` and `created` never change,
`sources` merges as a set, and when the two sides gave the note different
scopes the one that shares less wins, flagged `scope-clash`. Each merge is
recorded in [history](#history) as `merged`, so an automatic one is always
labelled, and two devices that merge the same versions write the same bytes.

When one passage was changed differently on both sides, bilbo keeps both in
the file:

```
<<<<<<< bilbo 9b46cd969c2b 2026-10-05T09:12-03:00
## Rollout

Ship on Monday.

======= bilbo b1fb38126e5c 2026-10-05T09:40-03:00
## Rollout

Ship on Friday.

>>>>>>> bilbo
```

Watch prints `notes/<file>: conflict in <n> passages; run bilbo check`.
`bilbo check` keeps reporting the conflict until the markers are gone, and the
digest labels a conflicted note `conflict`, an auto-merged one `auto-merged`,
and names open conflicts in a session's first digest. The `note` skill
resolves a conflict in the note it touches: one passage that keeps every fact
of both sides, and when the user said which side holds, only that side.

A resolution that leaves out a line one side held is reported by
`bilbo check` as `conflict: dropped <n> lines of '<heading path>', first
"<line>"; restore them or run bilbo sync declare <topic> "<why>"` until the
line is put back or declared dropped on purpose; a marker line that belongs to
no open conflict is `line <n>: stray conflict marker`:

```sh
bilbo sync declare release "Monday was superseded"
```

`<note>` is named as in `bilbo history`, and the reason is one line of up to
500 characters. It prints `declared plan-release.md: 3 lines dropped on
purpose`, writes only under `<root>/.bilbo/`, and syncs with the note. With
nothing to declare it exits 1. An agent that saves a note it read long ago
cannot revert a synced change in silence either: the first save after watch
wrote a note from another device is merged against the version from before
that write, and a merge that keeps both is flagged `stale-base`.

#### Status

`bilbo sync` prints, per syncing scope, where it syncs and which devices keep
up, then the notes that sync nowhere, the open conflicts and undeclared drops,
recent flags and changes of the device list:

```console
$ bilbo sync
scope personal file:///Users/me/Dropbox/bilbo: 212 notes, pushed 2026-10-05T09:12-03:00, pulled 2026-10-05T09:13-03:00
device personal rivendell: this device
device personal bagend: up to date
local: 188 notes sync nowhere
conflict notes/gotcha-nix.md: 1 passage
notice 2026-10-04T18:02-03:00 notes/plan-release.md: edit-beat-delete
change 2026-10-02T11:30-03:00 personal: device moria added by owner key (manifest 4)
```

A device's state is `up to date`, `behind by <n> segments`, or `stale since
<time>` once it left a segment of this device unacknowledged for
`sync.stale_days`, 180 by default. A `change` line is a device or an epoch
another device added: if you do not know the device, run `bilbo device revoke`.
`bilbo sync` reads only local state, so it works offline and changes nothing.
It exits 1 when a conflict or undeclared drop is open, a scope's folder was
unreachable or full, a scope or a device is stopped (its line goes to stderr,
and a segment that fails to verify names the copy to restore), or no
`bilbo watch` runs. Without a syncing scope it says
`no scope syncs; set scope.<name>.sync in <config path>` and exits 1.

#### The folder only grows

bilbo deletes nothing from the folder, and a new device reads every segment
from the first, so the folder and a new device's first sync grow with your
history. Trimming `history.keep_days` shortens only the local history. A
device that has left the scope's segments unacknowledged for `sync.stale_days`
stops holding back pruning of the versions it might still need as a merge
base.

Removing `<root>/.bilbo/` loses the local history and any manifest version not
yet on the folder (one this device wrote and no folder holds yet). Watch then
copies back the one scope of that name whose latest version lists this device,
reads the folder from the first segment and resumes where this device's own
files end. When the folder holds several scopes of that name it copies none and
says `run bilbo device recover on this device`. Two machines that share one device key are not supported: watch
stops pushing when it finds a segment of this device that its store did not
write.

#### Moving or ending sync

A scope's manifest pins the scheme of its URL, not a folder's path. To move a
synced folder, let the tool finish moving it, then change `scope.<name>.sync`
on every device at once; a device whose config still names the old path finds
no manifest of its scope there, and watch syncs nothing for that scope and
says `holds none of this device's scopes; if the folder moved, change
scope.<name>.sync on every device`.

To turn sync off for a scope, set `scope.<name>.sync = off` on each device. Each
keeps its notes and history; the folder keeps what it holds, which you can
delete once no device syncs through it. Delete the folder first, and the other
devices report it as not reachable.

#### What sync does not protect

- A stolen or lost device keeps every note and key it held. After
  [revoking it](#revoking-a-device), remove it from the cloud account that
  syncs the folder too: its sync client can still write there, and a version
  it signs that changes the device list shows up as a `change` line in
  `bilbo sync`.
- On a `file://` folder a revocation's new epoch is adopted only after the
  version has been read back unchanged for 10 minutes, so it takes effect up to
  10 minutes late.
- A device that holds the owner key can write a second scope named like one of
  yours. When `bilbo device recover` on an empty store names two scope ids,
  run `bilbo device list` afterwards and revoke a device you do not recognise.
- The folder shows file sizes, times and device ids, not names, topics or text.

### Library

The library holds sources: pages and documents an agent reads and cites, kept
as text. A **corpus** is a folder of sources on one subject,
`<root>/library/<corpus>/`, and a **source** is one Markdown file in it,
`<name>.md`, whatever its size. Corpus and source names are lowercase
kebab-case; a corpus is never called `show`, `stage`, `land`, `plan` or `read`,
and no source is called `guide`.

A source is not written by hand. Its frontmatter holds `id`, `fetched` (the
day the text was taken), `origin` (a quoted `url: ...` or `doc: ...`), and
`digest`, the SHA-256 of everything after the frontmatter, so `bilbo check`
sees any later edit. `kept` and `capture` are optional; a source fetched
from a URL carries no `capture` key. The body opens with
one `# ` title:

```markdown
---
id: 01M3EZ8NVEC2KJQNGK5DTK349R
fetched: 2026-08-23
origin: "url: https://go.dev/doc/effective_go"
digest: sha256:3c422834eb609821025bad23ee69ff3eec51facaa35dc7f6663d4304e1fcc34f
kept: 6-900
capture: external
---
# Effective Go

...
```

Each corpus also has a `guide.md`, the one file in it that agents edit: a
title, a lead on what the corpus grounds, and one `## <name>` entry of prose
per source. Sizes, token counts, dates and the catalog mark are never written
there; bilbo derives them when it prints the guide.

| Command | What it does |
| --- | --- |
| `bilbo library` | One row per corpus: its sources, size and guide title. |
| `bilbo library <corpus>` | The guide's path, then the guide with a facts line under each entry: id, size, tokens, `fetched`, headings. |
| `bilbo library show <corpus>/<name>\|<id>[#<anchor>] [--depth <n>]` | A source's header and one row per section: its lines, tokens and heading path. An anchor or `--depth` narrows the rows. |
| `bilbo library stage <url>` | Fetches the page, keeps what it answered, and prints the same as for a file. Changes nothing in the store. |
| `bilbo library stage <file> --origin "<url\|doc>: <value>" [--fetched <YYYY-MM-DD>] [--html]` | Copies a text file, with LF line endings, into the state folder and prints its lines, title and headings. `--html` converts a saved page. Changes nothing in the store. |
| `bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]` | Writes the source from the staged lines the ranges keep, adds its guide entry, and keeps the staged text under `<root>/.bilbo/captures/`. |
| `bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>] [--slice-lines <n>]` | Cuts the picks into slices and partitions, writes the plan under the state folder, and prints its id, the partitions and one row per slice. |
| `bilbo library read <plan> <slice>... [--part <k>/<n>]` | Prints the named slices with their line numbers, and logs the lines it printed. |

`bilbo library stage <url>` makes one GET request (at most 10 redirects, 60
seconds, 16 MiB) and sets the origin to `url: <url>` itself, so `--origin`,
`--fetched` and `--html` are usage errors with a URL. What the answer becomes
depends on its media type: `text/html` is converted to Markdown, any other
`text/*` is kept as text, and a PDF, an image or any other type is refused. For
a PDF the message gives the route: extract its text with a PDF tool, then stage
that file with `--origin "url: <url>"`. An unreachable server, a non-2xx
answer, a body that is not UTF-8 and a page that converts to no text are refused
too, and leave no stage. The conversion keeps navigation and footers; the agent
cuts them with the line ranges. A page saved from a browser is staged as a file
with `--html`, and a source landed from it still has `capture: external`.

A URL stage also keeps `raw`, the body as received, and `fetch.json`, the
request and answer (`url`, `final_url`, `status`, `media_type`, `fetched_at`,
`converter`), beside `capture.md`. `land` copies both into the capture folder
under `<root>/.bilbo/captures/`. Besides `stage:`, `capture:`, `lines:`,
`tokens:`, `title:` and `keep:`, a URL stage prints `raw:`, `media type:`,
`final url:` after a redirect, and `content:`, the lines of the page's one
`<main>` or `<article>` (`-` when it has no single one), which `title` and
`keep` then follow. `existing: <corpus>/<name>` names each source with the same
origin, so the agent can land with `--replace`. A capture that converts badly
warns on stderr and never refuses: `unclosed fence` (a code fence with no end),
`heading lost` (a page heading with no heading line in the capture, ten at
most) and `navigation suspect` (five or more lines of only links).

An agent adds a source in two steps, and never types its text. `stage` keeps
the text and shows where its headings are; the agent picks the line ranges
that are the page itself, not its navigation or footer, and `land` copies them.
`land` writes a source `bilbo check` accepts, but it asks for prose in the
guide: a new entry holds the line `TODO: describe this source.`, a new guide
`TODO: describe this corpus.`, and a source landed again with `--replace`
gets `stale: re-ingested <date>; ...` under its entry. `check` fails on every
one of those lines until the agent writes the entry and removes the line.

Before `land --replace` writes a source whose text changed, it checks every
`bilbo:` citation of that source in `<root>/notes/` against the old text and
the new one. A citation whose verdict changes to anything but `ok` (for
example `ok` to `quote_missing`, `anchor_missing` or `quote_elsewhere`) stops
the replace: nothing is written
and the stage is kept, and stderr gives the count and one
`notes/<file>:<line>: <old> -> <new>` line per citation. Fix the notes, or run
the same `land` with `--force` to replace anyway and keep those lines as
warnings. `--force` without `--replace` is a usage error. A citation that
already failed, and a citation in a guide, never blocks.

A source is read through a plan, so no tool's read cap decides where it ends.
`bilbo library plan` takes one or more picks, each `<corpus>/<name>` or an id,
with an optional `#<anchor>` that picks one section instead of the whole
source (the anchor is the heading path, or its tail, as `show` takes it). A
catalog is picked by section only, and two picks of one source may not share a
line. The plan cuts each pick at its section starts into **slices**, each sized
for one shell call: at most 24,000 bytes as `read` prints them, `--slice-bytes`
moves that between 1,000 and 30,000, and `--slice-lines` caps the lines too.
It then groups consecutive slices into **partitions** of at most
`--budget-tokens` (60,000 by default), each sized for one reader. The plan
file, `<plan>.json`, and the read log, `<plan>.log`, sit in `plans/` under the
state folder; a plan nothing has touched for 30 days is removed by the next
`library plan`.

`bilbo library read <plan> <slice>...` prints each slice under a header line
with the source's id and line range and an `in:` line with the heading path at
its first line, then every line as `<line>\t<text>`, then an end marker:

```text
-- slice 3/9: go/effective-go 01M3EZ8NVEC2KJQNGK5DTK349R lines 820-1104 --
-- in: Concurrency --
820	## Concurrency
...
1104	...
-- end slice 3/9 --
```

A read with no end marker, or with a gap in its line numbers, was cut by the
tool that ran it; read the slice again in parts with `--part 1/2` and
`--part 2/2` (up to `--part 8/8`), which print runs of nearly equal bytes. One
call that names several slices may not print more than the plan's slice size.
`read` appends the lines it printed to the plan's log, and refuses a slice
whose source was re-ingested after the plan, so make a new plan then.

To move or delete a source by hand, use `mv` or `rm` on the file, then edit the
guide: rename the entry's heading to match, or remove the entry. Then run
`bilbo check`, which says what is still out of step.

#### Citations

A claim from a note or a source is cited as
`bilbo:<id>#<anchor> "<quote>"`: `bilbo:`, the file's id, an optional `#` and
the heading path (or its tail) of the section, a space, then the quote in
double quotes. The quote is at least six words, copied from the text; markup,
spacing and curly quotes may differ, and `...` splits it into fragments that
must appear in order. A file with sections needs an anchor, and the title is
not one.

`bilbo cite [--plan <plan>]... [<file> | -]` reads a draft from the file or
stdin and prints one tab-separated row per citation (its line, the verdict,
the id with its anchor, the file's path or `-`, and a detail), then
`citations: <n> checked, <k> ok`. It reads no settings and writes nothing.

| Verdict | Meaning | Exit |
| --- | --- | --- |
| `ok` | The quote is in the anchored section. | 0 |
| `quote_elsewhere` | The quote is in the file, under other sections the detail names. | 0 |
| `ambiguous` | The anchor matches several sections and one holds the quote. | 0 |
| `too_short` | The quote has fewer than six words. | 0 |
| `quote_missing` | The quote is not in the file; the detail gives the nearest passage. | 1 |
| `anchor_missing` | No section matches the anchor. | 1 |
| `id_missing` | No note or source has the id, or two share it. | 1 |
| `unread` | With `--plan`: no match of the quote lies in lines the plan's log records as read. | 1 |

With `--plan <plan>`, given once or more, a citation of a source is `unread`
unless its quote sits in lines that `library read` printed for the source's
current text; notes and guides are never `unread`. Each plan then adds two
lines after the summary:

```text
coverage: plan <plan>: read 7 of 9 slices (<t> of <T> tokens); not read: go/effective-go lines 1200-1500 (slices 8-9)
picked: plan <plan>: go 2 of 14 sources (effective-go, errors#Wrapping)
```

`not read:` is `none` when every slice was read, and `picked:` counts the
sources each corpus holds now.

### History

Agents edit notes in place with their own tools, so bilbo sees no write. The
watcher, `bilbo watch`, runs in the background and records a version of a note
each time `<root>/notes/` has been quiet for 2 seconds after a change, or 10
seconds after the first change while edits keep coming. A burst of saves is one
version. It records a creation (`added`), an edit (`edited`), a rename
(`renamed`) and a deletion (`deleted`), keyed by the note's `id`, so a renamed
note is the same note. A change made while the watcher was not running is
recorded when it starts. It never touches `notes/`, apart from sweeping the
leftover of an interrupted restore (below) and writing what other devices
synced, when a scope [syncs](#sync). `bilbo setup` installs it as a
login service; see [Set up](#set-up).

History lives in `<root>/.bilbo/history/`, beside the notes. A version is a full
copy of the note, and identical content is stored once. Deleting
`<root>/.bilbo/history/` loses the history and nothing else: the next
`bilbo watch` starts over with every note `added`. Leave the rest of
`<root>/.bilbo/` alone, since it also holds the library's captures and the
state of [sync](#sync).

The watcher records only a regular, non-hidden file directly in `notes/`, named
`<kind>-<topic>.md`, no larger than 1 MiB, whose frontmatter has an `id`, and
which no other file shares. For a file skipped for its name, id, size or a
shared id it prints one line to stderr,
`bilbo: notes/<name>: not recorded: <reason>`. When `notes/`
cannot be listed, as when a sync tool swaps the folder, it records nothing and
waits; when it holds no note at all, it records no deletions. A second watcher
for the same store waits for the first to stop.

`bilbo history <note>` lists a note's versions, newest first, one per line as
`<version> <time> <event> <file name>`. `<note>` is its topic or its id; a
deleted note is still named by its last topic. A version is named by 6 or more
characters of the start of its id, and the list shows 12:

```console
$ bilbo history release
8f3c2a91d0b7 2026-10-03T16:02-03:00 edited decision-release.md
a1b2c3d4e5f6 2026-10-03T14:23-03:00 added decision-release.md
```

```sh
bilbo history release a1b2c3          # print that version's exact text
bilbo history release --diff a1b2c3   # diff it against the file on disk now
bilbo history release --diff a1b2c3 8f3c2a   # diff two versions
```

A diff is unified, with 3 lines of context, and its header lines are
`--- <file name>@<version>` and `+++ <file name>@<version>`, or
`+++ <file name>@now` against the file on disk. `bilbo history` changes nothing,
and when no watcher runs it adds `bilbo: bilbo watch is not running; recent
edits may not be recorded` on stderr.

`bilbo restore <note> <version>` writes a past version back to
`<root>/notes/<the version's file name>` and records it as a `restored` version:

```console
$ bilbo restore release a1b2c3
restored decision-release.md to a1b2c3d4e5f6
```

Nothing is lost on the way. Restore first records what the file holds now when
history does not, swaps the file in atomically, and records what came out of the
swap too, in case an agent wrote to the file meanwhile. If the version's file
name differs from the current one, the current file is removed, and restore
refuses with `<file name> is taken by another note` when another note holds that
name. It refuses a `deleted` version, and writes nothing when the file already
holds the version. It needs a filesystem that can swap two files atomically
(APFS, ext4, btrfs and xfs can). A restore that is killed leaves one hidden file,
`notes/.bilbo-restore-<id>`; the next restore or watcher scan records its bytes
if no version holds them, and deletes it. It works with or without a running
watcher.

Versions older than `history.keep_days` (90 by default) are dropped when the
watcher starts and every 24 hours after. The watcher reads the setting when it
starts, so after editing it, restart the watcher: `launchctl kickstart -k
gui/$(id -u)/io.github.delucca.bilbo.watch` on macOS, `systemctl --user restart
bilbo-watch.service` on Linux. Rerunning `bilbo setup` does not restart a watcher
it keeps. Kept are every newer version, each
note's newest version from before the cutoff, so the note as it stood then stays
readable, and for a deleted note its deletion and the version before it, however
old. Content that no kept version holds is removed from disk.

A secret pasted into a note stays in its history until pruning drops it. To
remove it sooner by hand:

1. Stop the watcher. `bilbo setup --yes --no-watch` removes its service, and
   `bilbo setup --yes` installs it again.
2. Delete `<root>/.bilbo/history/notes/<id>.jsonl`, with the note's `id` from its
   frontmatter. This drops every past version of that note.
3. Remove the secret from the note itself.
4. Start the watcher. It records the note as `added`, and its next prune removes
   the old content that no version holds.

### From an agent

The bilbo plugin gives Claude Code and Codex four skills, `note`, `recall`,
`reference` and `ingest`.

`note` writes what a later session should know. The agent runs it when you ask
to keep something ("note this", "save this as a decision"), or when the session
settled something a later one would otherwise work out again. It looks for the
note on the subject first with `bilbo recall`, and updates that note instead of
adding a second one. A new note comes from `bilbo new`; a note whose kind
changes is renamed with `mv -n`. It then runs `bilbo check`, fixes the lines
that name its own note, and reports the note's absolute path. When `check`
reports a [sync conflict](#conflicts) in that note, it replaces each block with
one passage that keeps every fact of both sides, unless you said which side
holds. Lines it dropped on purpose it declares with `bilbo sync declare`, and
the report names them. A conflict in a note it did not touch is left alone. It never invents
a source, and when `bilbo` is missing it says so and stops.

After each compaction, the plugin's `SessionStart` hook adds one line asking
the agent to save what the session settled, once the current task allows. It
prints nothing without `bilbo` on `PATH`.

`recall` runs `bilbo recall` with the user's words, retries twice in the note's likely
wording when nothing matches, and offers to open a hit. It searches through
`bilbo` only: when the binary is missing, it says so and stops.

When the user asks what the library says or names a corpus, `recall` searches
the library instead. It only locates sources: it hands each hit to `reference`
as a pick, and never answers from the snippets.

`reference` answers a question from the library ("what does the Go book say
about X"). It lists the corpora with `bilbo library`, reads the guides, and
posts its picks, the sources or sections that answer the question, before it
reads anything; a catalog is only ever picked by section, and a catalog, a bare
name or a question no guide entry covers is looked up with
`bilbo recall --library`. It plans the picks with `bilbo library plan` and
reads every slice through `bilbo library read`, never with a file tool. In Claude Code a plan of two to six partitions goes to
one `general-purpose` reader each, briefed by the skill's
`references/reader.md`; without the Agent tool, as in Codex, it plans smaller
slices and reads up to 100,000 tokens itself. It drafts one `bilbo:` citation
per claim, runs `bilbo cite --plan` until every verdict is `ok`, drops or
narrows any claim its quote does not support, and ends with the picks and
cite's `citations:`, `coverage:` and `picked:` lines, copied as printed.

`ingest` adds a source to the library ("ingest this page"). It stages the URL
with `bilbo library stage`, reads the capture around the suggested `keep` and
every navigation suspect, cuts the document out with `--keep` ranges and lands
it with `bilbo library land`. It then reads the landed source through
`bilbo library plan` and `read`, writes its guide entry, and runs `bilbo check`.
It reports the source's path, id and kept ranges. It never uses WebFetch: a page
bilbo cannot fetch is saved by the user and staged as a file, and a PDF goes
through `pdftotext -layout`. Files and PDF text are labelled `capture: external`.
It replaces an existing source only when you say so.

### The digest

`bilbo digest` is what the plugin's prompt hook runs, in Claude Code and in
Codex, on every prompt. It reads the hook's JSON from stdin and prints the
notes that bear on the prompt, which the tool hands to the agent as context:

```text
<!-- bilbo digest: 2 of 3 notes -->
Notes that may bear on this prompt (open the file to read more):
- /Users/me/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:13 (gotcha, 2026-10-03T13:31-03:00) SQLite needs a busy timeout > Fix: Set `busy_timeout = 5000` right after opening the connection.
- ...
(1 more passed; run bilbo recall for them)
```

A session's first digest lists up to 6 notes and later ones up to 3, never a
note the session already got, and never more than 9,000 bytes. When nothing
passes, it prints nothing.

A note passes only when it is close to the prompt. With an embedder, one of its
passages must reach `digest.min_similarity`; sharing words is not enough. Without
one, or when the embedder fails, is slow or has indexed nothing, a passage must
hold at least 3 of the prompt's words of four letters or more. The embedder gets
at most 1.2 seconds, and the whole run stays within 1.5. A prompt that starts with a path finds nothing.

`bilbo digest` always exits 0, so a failure never blocks a prompt: it prints
nothing and says why in one line on stderr. It remembers what each session was
shown in a file named after the session id, under `sessions/` in bilbo's cache
folder, and deletes those files after 30 days. It never changes the store or
the vector cache.

With `digest.log = on`, each run appends one JSON line to `digest.jsonl` under
bilbo's state folder (`$XDG_STATE_HOME/bilbo`, else `~/.local/state/bilbo`):
the session, the first 500 characters of the prompt, how the notes were
ranked, how many passed, which were shown and any error. The prompts are
plain text on disk, so the file is mode 0600 and off by default.

## Set up

`bilbo setup` plans every step, shows the plan, asks once, then applies it and
prints one line per step (`created`, `written`, `kept`, `installed`, `failed`,
and so on). It does these things:

- creates the store, `<root>/notes/`;
- writes the config, after checking the embedder with one real request;
- installs the bilbo plugin in Claude Code and Codex, at the binary's own
  version, for each of the two that is on your `PATH`;
- trusts the plugin's hooks in Codex, which runs a plugin hook only once
  it is trusted: setup asks `codex app-server` to record the trust, so no
  review step is left, and a release that changes the hook is trusted again on
  the next run;
- installs a timer that runs `bilbo index` every 15 minutes (a launchd agent on
  macOS, a systemd user timer on Linux), when an embedder is configured;
- installs a login service that runs `bilbo watch`, which records
  [note history](#history) and [syncs](#sync) the scopes that sync;
- checks each syncing scope's folder, and in the wizard can turn sync on.

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
| `--no-watch` | Skip the history watcher, and remove it when installed. |

The timer does not inherit your shell's environment, so it cannot read a key
from a variable. Keep the key in a file (`--embedder-token-file`, or paste it
in the wizard); with `--embedder-token-env` the timer step fails and says so.

### Watch service

Unless `--no-watch` is given or the wizard's answer declines it, setup installs
a login service that runs `bilbo watch`, restarts it when it exits and appends
its output to `watch.log` under bilbo's state folder: the launchd agent
`io.github.delucca.bilbo.watch` on macOS, the systemd user service
`bilbo-watch.service` on Linux. It needs no embedder and no network, and a key in
an environment variable does not fail it. The step is the `watch` line of the
report, after `timer`. In the wizard it is one question, "Record note history in
the background?", defaulting to yes.

### Sync step

The `sync` line of the report, after `watch`, checks each scope whose `sync` is
a URL: that this device has a key, that the watcher is wanted and that the
folder exists and is writable. It reports `ok: personal through
file:///Users/me/Dropbox/bilbo (212 notes)`, `failed: sync needs the watcher;
drop --no-watch`, `failed: <url> is not reachable: <reason>`, `skipped: no
device key; run bilbo device init in a terminal` or `skipped: no scope syncs`.
It writes nothing. In the wizard, after the watcher's question, one more asks
whether to sync notes between devices. On yes it asks for the scope and the
folder (absolute or starting with `~/`, its parent existing), and, when the
device has no key, whether you already have a recovery phrase: yes runs
`bilbo device recover`, no runs `bilbo device init`, with the same terminal
rules as [The ceremony](#the-ceremony). Nothing is written until you confirm
the summary. It then writes `scope.<name>.sync`, creates the folder's last
component when missing and writes the keys. Against a config that a module
manages, the wizard only shows the syncing scopes.

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

This unloads the timer, the watcher and the local embedder's service, removes the plugin
and its marketplace from Claude Code and Codex, and takes away Codex's trust of
the hooks. It keeps the store and its history, the
config, the key file and the downloaded model, and prints their paths. In a terminal
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
      "digest.log" = "on";
      "history.keep_days" = "30";
      "scope.work.paths" = "~/Developer/acme";
      "scope.work.embedder" = "local";
    };
    claude = "/Users/me/.local/bin/claude"; # null: look on the activation PATH
    codex = null;
    index = {
      enable = true; # the timer needs embedder.url
      every = 15;
    };
    watch.enable = true; # default; false passes --no-watch
  };
}
```

`settings` takes the keys in [Configuration](#configuration), the `sync.*` ones
included; scope keys are quoted attribute names, as above. Combining
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
| `digest.enable` | `off` turns [the digest](#the-digest) off: the hook prints nothing and writes nothing. `on` by default. |
| `digest.min_similarity` | How close a passage must be to enter the digest when an embedder answers, 0 to 1. Default 0.55. |
| `digest.log` | `on` appends each digest run to the digest log. `off` by default. |
| `scope.<name>.sync`, `scope.<name>.embedder`, `scope.<name>.paths`, `scope.<name>.marks` | Declare the scope `<name>`; see [Scopes](#scopes). `sync` is `off` or a [URL](#sync-urls), `embedder` is `any` or `local`, the others comma-separated lists. |
| `scope.default` | The declared scope `bilbo new` falls back on. |
| `sync.poll_seconds` | A whole number of seconds, 1 to 3600: how often `bilbo watch` looks for other devices' files; see [Sync](#sync). 30 by default. |
| `sync.stale_days` | A whole number of days, 1 to 3650: how long a device may leave a segment unacknowledged before `bilbo sync` calls it stale and pruning stops waiting for it. 180 by default. |
| `history.keep_days` | A whole number of days, 1 to 3650: the age past which `bilbo watch` prunes versions, under [the retention rule](#history). 90 by default. |

bilbo never prints the token. The vector cache lives under
`$XDG_CACHE_HOME/bilbo`, else `~/.cache/bilbo`; deleting it loses nothing that
`bilbo index` cannot rebuild.

### The embedder rule

A note whose scope sets `embedder = local` must not reach a remote embedder. An
unassigned note takes `local` as soon as any declared scope sets it, and
every note takes `any` when none does. When `embedder.url` is not on
`localhost`, `127.0.0.1` or `::1`, `bilbo index` sends no passage of a `local`
note, unless a note whose rule is `any` holds an identical passage, and drops
the vectors it already cached for them. It says so on stderr, counting distinct
inputs:

```console
$ bilbo index
embedded 1, kept 0, dropped 0
bilbo: withheld 2 passages from http://bagend:8081: their scope allows only a loopback embedder
```

`recall` and the digest still reach withheld notes, by keywords, and `recall`
does not count them as not indexed. The digest admits a withheld passage on its
keyword gate, so a store withheld whole still reaches the digest without a
request to the embedder.

Declaring the first `embedder = local` scope withholds every note not yet in a
scope. Triage the store first, as in [Scopes](#scopes), or run
[`--embedder-local`](#local-embedder), under which nothing is withheld. The
rule trusts the URL's host as written: an ssh tunnel on `localhost` counts as
local, and the text leaves the machine through it. A loopback embedder is
reached directly: bilbo ignores the proxy variables (`HTTP_PROXY`,
`HTTPS_PROXY`, `ALL_PROXY` and their lowercase forms) for it.

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
