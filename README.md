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
`doc` or `search`. No other key is allowed. The store root is `$BILBO_HOME`,
else `$XDG_DATA_HOME/bilbo`, else `~/.local/share/bilbo`, on macOS too.

### Commands

| Command | What it does |
| --- | --- |
| `bilbo new <kind> <topic> [--title <text>]` | Creates the note and prints its path. Exits 1 when the topic already has a note. |
| `bilbo check` | Prints every problem in the store, one per line, and changes nothing. Exits 1 when it finds any. |
| `bilbo recall <query>... [--kind <kind>]... [--limit <n>]` | Prints the notes that best match, best first, 10 by default. Exits 1 when nothing matches. |
| `bilbo recall <query>... --library [--corpus <corpus>]... [--limit <n>]` | Prints the sources and guides of the library that best match, by keyword, one block per file. Exits 1 when nothing matches. |
| `bilbo index` | Embeds the passages the vector cache lacks and drops the ones no note holds any more. |
| `bilbo digest` | Run by the plugin's prompt hook; see [The digest](#the-digest). |
| `bilbo library` | Lists and shows the library, and stages and lands its sources; see [Library](#library). |
| `bilbo cite [--plan <plan>]... [<file> \| -]` | Checks every `bilbo:` citation in a draft; see [Citations](#citations). |
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

### From an agent

The bilbo plugin gives Claude Code and Codex four skills, `note`, `recall`,
`reference` and `ingest`.

`note` writes what a later session should know. The agent runs it when you ask
to keep something ("note this", "save this as a decision"), or when the session
settled something a later one would otherwise work out again. It looks for the
note on the subject first with `bilbo recall`, and updates that note instead of
adding a second one. A new note comes from `bilbo new`; a note whose kind
changes is renamed with `mv -n`. It then runs `bilbo check`, fixes the lines
that name its own note, and reports the note's absolute path. It never invents
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
and so on). It does five things:

- creates the store, `<root>/notes/`;
- writes the config, after checking the embedder with one real request;
- installs the bilbo plugin in Claude Code and Codex, at the binary's own
  version, for each of the two that is on your `PATH`;
- trusts the plugin's hooks in Codex, which runs a plugin hook only once
  it is trusted: setup asks `codex app-server` to record the trust, so no
  review step is left, and a release that changes the hook is trusted again on
  the next run;
- installs a timer that runs `bilbo index` every 15 minutes (a launchd agent on
  macOS, a systemd user timer on Linux), when an embedder is configured.

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

The timer does not inherit your shell's environment, so it cannot read a key
from a variable. Keep the key in a file (`--embedder-token-file`, or paste it
in the wizard); with `--embedder-token-env` the timer step fails and says so.

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

This unloads the timer and the local embedder's service, removes the plugin
and its marketplace from Claude Code and Codex, and takes away Codex's trust of
the hooks. It keeps the store, the config,
the key file and the downloaded model, and prints their paths. In a terminal
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
    };
    claude = "/Users/me/.local/bin/claude"; # null: look on the activation PATH
    codex = null;
    index = {
      enable = true; # the timer needs embedder.url
      every = 15;
    };
  };
}
```

`settings` takes the keys in [Configuration](#configuration). Combining
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

bilbo never prints the token. The vector cache lives under
`$XDG_CACHE_HOME/bilbo`, else `~/.cache/bilbo`; deleting it loses nothing that
`bilbo index` cannot rebuild.

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
