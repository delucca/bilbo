# The library

Add a page or a document to the library as a source, read it in full without a
tool's read cap, and cite it. An agent does most of this through the `ingest`
and `reference` skills; this page is what those skills run.

The library holds sources: pages and documents an agent reads and cites, kept as
text. A corpus is a folder of sources on one subject, and a source is one
Markdown file in it. The file formats are in [Files and
formats](../reference/files.md#library-files).

## Commands at a glance

| Command | What it does |
| --- | --- |
| `bilbo library` | One row per corpus: its sources, size and guide title. |
| `bilbo library <corpus>` | The guide's path, then the guide with a facts line under each entry: id, size, tokens, `fetched`, headings. On a terminal: the lead and each entry with its facts, sizes and tokens grouped. |
| `bilbo library show <corpus>/<name>\|<id>[#<anchor>] [--depth <n>]` | A source's header and one row per section: its lines, tokens and heading path. An anchor or `--depth` narrows the rows. |
| `bilbo library stage <url>` | Fetches the page, keeps what it answered, and prints the same as for a file. Changes nothing in the store. |
| `bilbo library stage <file> --origin "<url\|doc>: <value>" [--fetched <YYYY-MM-DD>] [--html]` | Copies a text file, with LF line endings, into the state folder and prints its lines, title and headings. `--html` converts a saved page. Changes nothing in the store. |
| `bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]... [--title <text>] [--replace [--force]]` | Writes the source from the staged lines the ranges keep, adds its guide entry, and keeps the staged text under `<root>/.bilbo/captures/`. |
| `bilbo library plan <ref>... [--budget-tokens <n>] [--slice-bytes <n>] [--slice-lines <n>]` | Cuts the picks into slices and partitions, writes the plan under the state folder, and prints its id, the partitions and one row per slice. |
| `bilbo library read <plan> <slice>... [--part <k>/<n>]` | Prints the named slices with their line numbers, and logs the lines it printed. |

## Add a source

An agent adds a source in two steps, and never types its text. `stage` keeps the
text and shows where its headings are; the agent picks the line ranges that are
the page itself, not its navigation or footer, and `land` copies them.

### Stage a page

`bilbo library stage <url>` makes one GET request (at most 10 redirects, 60
seconds, 16 MiB) and sets the origin to `url: <url>` itself, so `--origin`,
`--fetched` and `--html` are usage errors with a URL. What the answer becomes
depends on its media type: `text/html` is converted to Markdown, any other
`text/*` is kept as text, and a PDF, an image or any other type is refused. For
a PDF the message gives the route: extract its text with a PDF tool, then stage
that file with `--origin "url: <url>"`. An unreachable server, a non-2xx answer,
a body that is not UTF-8 and a page that converts to no text are refused too,
and leave no stage. The conversion keeps navigation and footers; the agent cuts
them with the line ranges. A page saved from a browser is staged as a file with
`--html`, and a source landed from it still has `capture: external`.

A URL stage also keeps `raw`, the body as received, and `fetch.json`, the
request and answer (`url`, `final_url`, `status`, `media_type`, `fetched_at`,
`converter`), beside `capture.md`. `land` copies both into the capture folder
under `<root>/.bilbo/captures/`. Besides `stage:`, `capture:`, `lines:`,
`tokens:`, `title:` and `keep:`, a URL stage prints `raw:`, `media type:`,
`final url:` after a redirect, and `content:`, the lines of the page's one
`<main>` or `<article>` (`-` when it has no single one), which `title` and
`keep` then follow. `existing: <corpus>/<name>` names each source with the same
origin, so the agent can land with `--replace`.

A capture that converts badly warns on stderr and never refuses:

- `unclosed fence`: a code fence with no end.
- `heading lost`: a page heading with no heading line in the capture, ten at
  most.
- `navigation suspect`: five or more lines of only links.

### Land the source

`bilbo library land <stage> <corpus>/<name> --keep <a>-<b>[,<c>-<d>]...` writes
the source from the lines the ranges keep. `land` writes a source `bilbo check`
accepts, but it asks for prose in the guide: a new entry holds the line `TODO:
describe this source.`, a new guide `TODO: describe this corpus.`, and a source
landed again with `--replace` gets `stale: re-ingested <date>; ...` under its
entry. `check` fails on every one of those lines until the agent writes the
entry and removes the line.

### Replace a source

Land again with `--replace`. Before `land --replace` writes a source whose text
changed, it checks every `bilbo:` citation of that source in `<root>/notes/`
against the old text and the new one. A citation whose verdict changes to
anything but `ok` (for example `ok` to `quote_missing`, `anchor_missing` or
`quote_elsewhere`) stops the replace: nothing is written and the stage is kept,
and stderr gives the count and one `notes/<file>:<line>: <old> -> <new>` line
per citation.

Fix the notes, or run the same `land` with `--force` to replace anyway and keep
those lines as warnings. `--force` without `--replace` is a usage error. A
citation that already failed, and a citation in a guide, never blocks.

## Read a source

A source is read through a plan, so no tool's read cap decides where it ends.

### Plan the read

`bilbo library plan` takes one or more picks, each `<corpus>/<name>` or an id,
with an optional `#<anchor>` that picks one section instead of the whole source
(the anchor is the heading path, or its tail, as `show` takes it). A catalog is
picked by section only, and two picks of one source may not share a line.

The plan cuts each pick at its section starts into slices, each sized for one
shell call: at most 24,000 bytes as `read` prints them, `--slice-bytes` moves
that between 1,000 and 30,000, and `--slice-lines` caps the lines too. It then
groups consecutive slices into partitions of at most `--budget-tokens` (60,000
by default), each sized for one reader. The plan file, `<plan>.json`, and the
read log, `<plan>.log`, sit in `plans/` under the state folder; a plan nothing
has touched for 30 days is removed by the next `library plan`.

### Read the slices

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
tool that ran it; read the slice again in parts with `--part 1/2` and `--part
2/2` (up to `--part 8/8`), which print runs of nearly equal bytes. One call that
names several slices may not print more than the plan's slice size. `read`
appends the lines it printed to the plan's log, and refuses a slice whose source
was re-ingested after the plan, so make a new plan then.

## Move or delete a source

Use `mv` or `rm` on the file, then edit the guide: rename the entry's heading to
match, or remove the entry. Then run `bilbo check`, which says what is still out
of step.

## Citations

Cite a claim from a note or a source so that bilbo can check it. The syntax,
`bilbo:<id>#<anchor> "<quote>"`, is in [Files and
formats](../reference/files.md#citation-syntax).

`bilbo cite [--plan <plan>]... [<file> | -]` reads a draft from the file or
stdin and prints one tab-separated row per citation (its line, the verdict, the
id with its anchor, the file's path or `-`, and a detail), then `citations: <n>
checked, <k> ok`. It reads no settings and writes nothing.

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

### Check coverage with a plan

With `--plan <plan>`, given once or more, a citation of a source is `unread`
unless its quote sits in lines that `library read` printed for the source's
current text; notes and guides are never `unread`. Each plan then adds two lines
after the summary:

```text
coverage: plan <plan>: read 7 of 9 slices (<t> of <T> tokens); not read: go/effective-go lines 1200-1500 (slices 8-9)
picked: plan <plan>: go 2 of 14 sources (effective-go, errors#Wrapping)
```

`not read:` is `none` when every slice was read, and `picked:` counts the
sources each corpus holds now.

## See also

- [What your agent does](agents.md) for the `ingest` and `reference` skills that
  run these commands.
- [Files and formats](../reference/files.md#library-files) for the source and
  guide formats.
- [Commands](../reference/commands.md#library)
