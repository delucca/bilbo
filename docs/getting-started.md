# Getting started

This tutorial takes you from nothing to a working bilbo: you install it, set it
up, write a first note, find it again and see your agent use it. It takes about
ten minutes.

You need macOS (arm64 or Intel) or Linux (x86_64 or arm64, glibc), and Claude
Code or Codex if you want the agent steps.

## 1. Install bilbo

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/delucca/bilbo/releases/latest/download/bilbo-installer.sh | sh
```

The binary goes to `$XDG_BIN_HOME` when it is set, else `~/.local/bin`. Check
that your shell finds it:

```console
$ bilbo --version
bilbo <version>
```

If the shell says `command not found`, open a new terminal so it reads the PATH
change the installer made. [Install](install.md) covers what the installer
writes, Nix and upgrades.

## 2. Run setup

```sh
bilbo setup
```

In a terminal, `bilbo setup` starts a wizard. It asks which embedder to use for
search by meaning. For this tutorial, pick the first choice, no embedder, which
gives keyword search only. You can add an embedder later with [the embedders
guide](guides/embedders.md).

The wizard then shows its plan and asks once before it writes anything. Say yes.
A real run creates the store, writes the config, installs the plugin in each of
Claude Code and Codex that is on your `PATH`, and installs a login service that
records note history. It installs the index timer only when an embedder is
configured.

To show you the result without touching your machine, the output below comes
from a run in a scratch folder with `--yes --no-plugin --no-timer --no-watch`,
which skips those installs. On a real run the `claude`, `codex`, `hook` and
`watch` lines say `installed` instead; without an embedder the rest stay
`skipped`.

```console
$ bilbo setup --yes --no-plugin --no-timer --no-watch
store created: /Users/me/.local/share/bilbo/notes
config written: /Users/me/.config/bilbo/config
key skipped: no embedder
model skipped: not local
server skipped: not local
embedder skipped: none configured
claude skipped: --no-plugin
codex skipped: --no-plugin
hook skipped: no codex plugin
timer skipped: --no-timer
watch skipped: --no-watch
sync skipped: no scope syncs
```

One line per step. `created` and `written` mean setup made something; `kept`
means it was already right; `failed` means a step went wrong (see
[Troubleshooting](troubleshooting.md)). Run setup again at any time: with the
same inputs, every step that did something is `kept`.

[Set up](guides/setup.md) lists every step and option.

## 3. Write your first note

A note is one Markdown file named `<kind>-<topic>.md`. `bilbo new` creates it
with the frontmatter and a title:

```console
$ bilbo new gotcha sqlite-busy-timeout --title "SQLite needs a busy timeout"
/Users/me/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md
```

It prints the path of the new file. Open that file and write under the title:

```markdown
---
id: 01M494RX6VZGVSP83V7K66596B
created: 2026-10-06T14:39-03:00
---

# SQLite needs a busy timeout

Two writers on the same database file fail with `SQLITE_BUSY` unless each
connection sets `PRAGMA busy_timeout`.

## Fix

Set `busy_timeout = 5000` right after opening the connection.
```

Your `id` and `created` differ. Check that the store is well formed:

```console
$ bilbo check
```

Nothing printed and exit code 0 means no problems.
[Files](reference/files.md#note-format) describes the note format and the kinds.

## 4. Recall it

```console
$ bilbo recall busy timeout
/Users/me/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:11	gotcha	2026-10-06T14:39-03:00
SQLite needs a busy timeout > Fix
Set `busy_timeout = 5000` right after opening the connection.
```

You see one block per matching note: the path and line of the best passage, the
kind and `created` (tab-separated), the heading path, then the start of the
passage. A query that matches nothing prints `bilbo: no notes match` on stderr
and exits 1.

Without an embedder, `recall` matches whole words, ignoring case and accents.

## 5. See your agent use it

With the plugin installed, your agent has four skills: `note`, `recall`,
`reference` and `ingest`.

1. Start a new Claude Code or Codex session in any project.
2. Ask it to "recall" what you know about SQLite timeouts. It runs `bilbo
   recall` with your words and offers to open a hit.
3. Ask it to "note this" after it settles something. It looks for a note on the
   subject first, then updates that note or creates one with `bilbo new`, runs
   `bilbo check` and reports the note's absolute path.

You also get the digest without asking. On every prompt, the plugin hands the
agent the notes that bear on it, before it starts:

```text
<!-- bilbo digest: 1 of 1 notes -->
Notes that may bear on this prompt (open the file to read more):
- /Users/me/.local/share/bilbo/notes/gotcha-sqlite-busy-timeout.md:11 (gotcha, 2026-10-06T14:39-03:00) SQLite needs a busy timeout > Fix: Set `busy_timeout = 5000` right after opening the connection.
```

If you ask for something that note bears on, expect this block at the top of the
agent's context. [Agents](guides/agents.md) explains each skill, the compaction
reminder and the digest rules.

## Where next

- Search by meaning, so `recall` finds notes that share no word with your query:
  [Embedders](guides/embedders.md).
- Understand the parts: [Concepts](concepts.md).
- Split your notes by project: [Scopes](guides/scopes.md).
- Undo a careless edit: [History](guides/history.md).
- Add documentation as sources your agent can cite:
  [Library](guides/library.md).
- Keep notes in step across your machines: [Sync](guides/sync.md).
- Something went wrong: [Troubleshooting](troubleshooting.md).
