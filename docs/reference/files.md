# Files and formats

The files bilbo reads and writes: the note, the store that holds it, the
library's sources and guides, and the citation syntax.

## Note format

A note is one Markdown file in `<root>/notes/`, named `<kind>-<topic>.md`. The
kind is one of `plan`, `spec`, `design`, `decision`, `gotcha`, `research`,
`review`, `report` or `reference`. The topic is lowercase kebab-case, and a
topic has at most one note whatever its kind. `bilbo new` writes the frontmatter
and the title; the agent writes the rest:

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

The frontmatter keys:

| Key | Meaning |
| --- | --- |
| `id` | A ULID. History and citations identify the note by it, so a renamed note is the same note. |
| `created` | The local time to the minute. |
| `sources` | Optional. Each item is a quoted `<type>: <value>` with the type `url`, `code`, `doc` or `search`. |
| `scope` | Optional. A name from [Scopes](../guides/scopes.md). |

No other key is allowed. The title is one `# ` line after the frontmatter.
`bilbo check` reports every note that breaks these rules.

## Store layout

The store root is the root; see [Folders](configuration.md#folders). It holds:

```
<root>/
  notes/                one note per topic: <kind>-<topic>.md
  library/<corpus>/     guide.md and one <name>.md per source
  .bilbo/
    history/            the versions of every note
    captures/           the staged text of every landed source
    scopes/             the manifests and segments of syncing scopes
```

- `<root>/.bilbo/history/` holds [note history](../guides/history.md). Deleting
  it loses the history and nothing else.
- `<root>/.bilbo/captures/` holds what `bilbo library land` keeps of each staged
  page.
- `<root>/.bilbo/scopes/` holds what [sync](../guides/sync.md) keeps for each
  scope. Leave the rest of `<root>/.bilbo/` alone.

The vector cache, the digest's session files and the downloaded model live in
the cache folder, and the device keys, plans and logs in the state folder; see
[Configuration](configuration.md#folders).

## Library files

The library holds sources: pages and documents an agent reads and cites, kept as
text. A corpus is a folder of sources on one subject,
`<root>/library/<corpus>/`, and a source is one Markdown file in it,
`<name>.md`, whatever its size. A corpus folder holds its `guide.md` and its
sources, and nothing else. Corpus and source names are lowercase kebab-case; a
corpus is never called `show`, `stage`, `land`, `plan` or `read`, and no source
is called `guide`.

A source is not written by hand. Its frontmatter holds `id`, `fetched` (the day
the text was taken), `origin` (a quoted `url: ...` or `doc: ...`), and `digest`,
the SHA-256 of everything after the frontmatter, so `bilbo check` sees any later
edit. `kept` and `capture` are optional; a source fetched from a URL carries no
`capture` key. The body opens with one `# ` title:

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

Each corpus also has a `guide.md`, the one file in it that agents edit: a title,
a lead on what the corpus grounds, and one `## <name>` entry of prose per
source. Sizes, token counts, dates and the catalog mark are never written there;
bilbo derives them when it prints the guide.

How sources get into the library, and how to read them, is in [The
library](../guides/library.md).

## Citation syntax

A claim from a note or a source is cited as `bilbo:<id>#<anchor> "<quote>"`:
`bilbo:`, the file's id, an optional `#` and the heading path (or its tail) of
the section, a space, then the quote in double quotes. The quote is at least six
words, copied from the text; markup, spacing and curly quotes may differ, and
`...` splits it into fragments that must appear in order. A file with sections
needs an anchor, and the title is not one.

The verdicts `bilbo cite` gives a citation are in
[Citations](../guides/library.md#citations).
