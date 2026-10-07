# library-ingest Specification

## Purpose
`bilbo library stage` and `bilbo library land` put a source into the library without an agent typing its text: bilbo keeps the staged text, the agent names the line ranges to keep, and bilbo writes the body, its digest and the guide entry.

## Requirements

### Requirement: Stage a file
`bilbo library stage <file> --origin "<url|doc>: <value>" [--fetched <YYYY-MM-DD>] [--html]` SHALL read the file as UTF-8 text, drop a leading byte order mark, turn every CRLF and every lone CR into LF, add a final newline when it lacks one, and write the result as `capture.md` in a new folder `<state>/bilbo/staging/<stage>/`, where `<state>` is the state folder `bilbo --help` names and `<stage>` a fresh ULID. With `--html`, `capture.md` SHALL instead hold the file's text converted by the `library-fetch` HTML conversion rules, then normalized the same way, and the folder SHALL also hold the file's bytes as `raw`. `--fetched` SHALL default to today's local date. Staging SHALL change nothing under the store root.

#### Scenario: A file is staged
- **WHEN** an agent runs `bilbo library stage /tmp/spec.txt --origin "url: https://go.dev/ref/spec"` on a file with CRLF line endings
- **THEN** the exit code is 0, `<state>/bilbo/staging/<stage>/capture.md` holds the file's text with LF line endings, and no entry under the store root is created or changed

#### Scenario: The origin and date are kept for landing
- **WHEN** an agent stages a file with `--origin "doc: The Go Programming Language, chapter 8" --fetched 2026-08-23` and then lands it
- **THEN** the source has `origin: "doc: The Go Programming Language, chapter 8"` and `fetched: 2026-08-23`

#### Scenario: A saved page is converted
- **WHEN** an agent runs `bilbo library stage /tmp/page.html --html --origin "url: https://platform.example.com/docs/agents"` on a file holding `<h1>Agents</h1><p>Text.</p>`
- **THEN** `capture.md` is `# Agents`, a blank line and `Text.`, `raw` holds the file's bytes, and a source landed from it has `capture: external`

#### Scenario: HTML without the flag stays as written
- **WHEN** an agent stages the same file without `--html`
- **THEN** `capture.md` holds the HTML text as it was, and the folder holds no `raw`

### Requirement: Stage output
`bilbo library stage` SHALL print to stdout, in this order:
- `stage: <stage>` and `capture: <absolute path of capture.md>`;
- `raw: <absolute path>` when the stage holds `raw`;
- for a URL, `media type: <type>`, or `-` with none, then `final url: <url>` when redirects were followed;
- for a capture converted from HTML, `content: <a>-<b>`, or `content: -` when there are no content lines;
- one `existing: <corpus>/<name>` line per source whose `origin` equals the stage's, in name order;
- `lines: <n>`, `tokens: <n>`, `title: <text>` and `keep: <a>-<b>`;
- a blank line, then one row per level-1 or level-2 heading outside fenced code blocks: its line number, a tab and the line as written.

Content lines are those holding the conversion of the page's only `<main>` element, or with no `<main>` its only `<article>`, when that text occurs exactly once in the capture. `title` is the text of the first level-1 heading within the content lines, else the capture's first, else `-`. `keep` starts on the line after the title when the title is a content line or there are no content lines, else on the first content line, or with no title on the first non-blank line. It ends on the last non-blank line of the content lines, else of the capture, and is `-` when that range is empty.

#### Scenario: A page with a title and a footer
- **WHEN** the capture has navigation on lines 1 to 4, `# Effective Go` on line 5, text to line 900 and blank lines after it
- **THEN** stdout holds `title: Effective Go` and `keep: 6-900`, and a row `5`, a tab and `# Effective Go`

#### Scenario: Text with no title
- **WHEN** the capture has no level-1 heading and its text runs from line 1 to line 40
- **THEN** stdout holds `title: -` and `keep: 1-40`

#### Scenario: A fetched page with a main element
- **WHEN** a fetched page converts to 503 lines, its site title `# The Cargo Book` on line 20, and its `<main>` to lines 24 to 499, which open with `# The Manifest Format`
- **THEN** stdout holds `media type: text/html`, `content: 24-499`, `title: The Manifest Format` and `keep: 25-499`

#### Scenario: A page without a main element
- **WHEN** a fetched page has two `<article>` elements and no `<main>`
- **THEN** stdout holds `content: -`, and `title` and `keep` follow the capture's first level-1 heading

#### Scenario: A redirect is shown
- **WHEN** the staged URL redirected to `https://example.org/b`
- **THEN** stdout holds `final url: https://example.org/b`

#### Scenario: A re-ingest is noticed
- **WHEN** `go/effective-go` has `origin: "url: https://go.dev/doc/effective_go"` and an agent stages `https://go.dev/doc/effective_go`
- **THEN** stdout holds `existing: go/effective-go`

#### Scenario: A file has no fetch lines
- **WHEN** an agent stages a text file without `--html`
- **THEN** stdout holds no `raw:`, `media type:`, `final url:` or `content:` line

### Requirement: Stage refusals
A file that is missing, unreadable, a folder, not valid UTF-8, or holds only whitespace SHALL make `stage` exit 1 with the reason on stderr and leave no stage folder. A missing `--origin`, an origin that breaks the `library-store` origin rule, or a `--fetched` that is not a real `YYYY-MM-DD` date SHALL be a usage error. When neither `XDG_STATE_HOME` nor `HOME` is an absolute path, `stage` SHALL exit 2 naming them.

#### Scenario: Not text
- **WHEN** an agent stages a PDF file
- **THEN** stderr says the file is not valid UTF-8, the exit code is 1, and no new folder exists under `<state>/bilbo/staging/`

#### Scenario: No origin
- **WHEN** an agent runs `bilbo library stage /tmp/spec.txt`
- **THEN** bilbo prints a message naming `--origin` to stderr and exits 2

#### Scenario: A bad origin type
- **WHEN** an agent passes `--origin "web: https://go.dev"`
- **THEN** bilbo prints a message naming the origin to stderr and exits 2

### Requirement: Land a source
`bilbo library land <stage> <corpus>/<name> --keep <ranges> [--title <text>]` SHALL write the source `<root>/library/<corpus>/<name>.md`, creating `<root>/library/` and the corpus folder when missing. The source SHALL have a fresh `id`, the `fetched` and `origin` the stage recorded, the `digest` of the body it builds, `kept` unless the ranges cover every line of the capture, `capture: external` for a staged file, with or without `--html`, and no `capture` key for a staged URL. On success, `land` SHALL print `source: <absolute path>`, `id: <id>`, `guide: <absolute path>` and `capture folder: <absolute path>` to stdout, remove the stage folder, and exit 0.

#### Scenario: A first source in a new corpus
- **WHEN** an agent stages a 900-line file and runs `bilbo library land <stage> go/effective-go --keep 6-900`
- **THEN** `<root>/library/go/effective-go.md` exists with a fresh id, `kept: 6-900` and `capture: external`, stdout holds the four lines, the stage folder is gone, and the exit code is 0

#### Scenario: The whole capture is kept
- **WHEN** the capture has 40 lines and the agent passes `--keep 1-40 --title "Errors"`
- **THEN** the source has no `kept` key

#### Scenario: A landed source passes check
- **WHEN** an agent lands a source into an otherwise valid store, then writes its guide entry and removes the `TODO` line
- **THEN** `bilbo check` exits 0

#### Scenario: A fetched page has no capture label
- **WHEN** an agent stages `https://go.dev/doc/effective_go` and lands it as `go/effective-go`
- **THEN** the source has no `capture` key, and its facts line in `bilbo library go` has no ` · capture ` part

### Requirement: Keep ranges
`--keep` SHALL be required. It takes 1-based inclusive ranges `<a>-<b>` joined by commas and MAY be given several times. All ranges together, in the order given, SHALL be ascending and not overlapping, with `a` not above `b`, and inside the capture's lines. `land` SHALL write them as `kept`, merging ranges that touch. Any other `--keep` SHALL be a usage error that writes nothing.

#### Scenario: Two cuts
- **WHEN** an agent passes `--keep 6-400 --keep 420-900`
- **THEN** the source has `kept: 6-400,420-900` and its body holds those lines in order

#### Scenario: Ranges that touch
- **WHEN** an agent passes `--keep 6-400,401-900`
- **THEN** the source has `kept: 6-900`

#### Scenario: A range past the end
- **WHEN** the capture has 900 lines and an agent passes `--keep 6-950`
- **THEN** bilbo prints a message naming `--keep` to stderr, exits 2 and writes no file

#### Scenario: Overlapping ranges
- **WHEN** an agent passes `--keep 6-400,300-900`
- **THEN** bilbo exits 2 and writes no file

### Requirement: The body land builds
The body SHALL be `# <title>`, a blank line, then the kept lines in order, each ending with a newline. The title SHALL be `--title`, or else the text of the capture's first level-1 heading; with neither, `land` SHALL be a usage error. When the kept lines hold a level-1 heading outside fenced code blocks, `land` SHALL add one `#` in front of every heading line outside fences in them and print a line saying so to stderr. Nothing else SHALL change a kept line.

#### Scenario: Kept lines as they were
- **WHEN** the kept lines hold trailing spaces, tabs, an HTML comment and a fenced block
- **THEN** the body after the title and the blank line equals those lines byte for byte

#### Scenario: The title comes from the capture
- **WHEN** the capture's first level-1 heading is `# Effective Go` and no `--title` is given
- **THEN** the body starts with `# Effective Go`

#### Scenario: Headings are demoted under a kept H1
- **WHEN** the kept lines hold `# Part one` and `## Details`, and the agent passes `--title "Book"`
- **THEN** the body starts with `# Book` and holds `## Part one` and `### Details`, and stderr has one line saying the headings were demoted

#### Scenario: No title at all
- **WHEN** the capture has no level-1 heading and no `--title` is given
- **THEN** bilbo prints a message naming `--title` to stderr, exits 2 and writes no file

#### Scenario: An unusable title
- **WHEN** an agent passes `--title ""` or a title holding a line break
- **THEN** bilbo exits 2 and writes no file

### Requirement: The guide entry land writes
When `land` writes a new source, it SHALL add the entry to the corpus's guide. A missing guide SHALL be created first, holding a fresh `id`, `created` set to now, `# <corpus>`, a blank line and `TODO: describe this corpus.`. The entry SHALL be appended as a blank line, `## <name>`, a blank line and `TODO: describe this source.`. When the guide already has `## <name>`, its prose SHALL be kept and the stale line SHALL be put directly under the heading instead.

#### Scenario: A new entry
- **WHEN** an agent lands `go/errors` into a corpus whose guide has two entries
- **THEN** the guide ends with `## errors`, a blank line and `TODO: describe this source.`, and the two entries are unchanged

#### Scenario: A new corpus gets a guide
- **WHEN** an agent lands `haskell/strings` and `<root>/library/haskell/` does not exist
- **THEN** `library/haskell/guide.md` holds a fresh `id`, a `created` time, `# haskell`, `TODO: describe this corpus.` and the `## strings` entry with its stub

#### Scenario: An entry that outlived its source
- **WHEN** the guide still has `## errors` with prose, and `errors.md` does not exist
- **THEN** after `land`, the prose is still there, and the line under `## errors` is `stale: re-ingested <today>; re-read the source and revise this entry.`

### Requirement: Replace a source
With `--replace`, `land` SHALL replace an existing source: it keeps the old `id` and writes every other key and the body anew from the stage. When the new digest differs from the old one, it SHALL first run the citation pre-check, then put `stale: re-ingested <YYYY-MM-DD>; re-read the source and revise this entry.`, with today's local date, directly under the entry's heading, replacing a stale line already there, or add the entry with its stub when there is none. When the digest is the same, the guide SHALL be left as it is. Without `--replace`, an existing source SHALL make `land` exit 1, naming the file and `--replace`; with `--replace`, a missing source SHALL make it exit 1.

#### Scenario: A re-ingest with new text
- **WHEN** `go/effective-go` exists with id `01M3EZ8NVEC2KJQNGK5DTK349R`, no note cites it, and an agent lands a new stage with `--replace` whose body differs
- **THEN** the source keeps that id, has the new `fetched`, `digest` and body, and the line under `## effective-go` is the stale line with today's date, followed by the old prose

#### Scenario: A re-ingest with the same text
- **WHEN** the new body is byte for byte the old one
- **THEN** the source has the new `fetched`, and `guide.md` has the same bytes as before

#### Scenario: A taken name
- **WHEN** `go/effective-go` exists and an agent runs `land` without `--replace`
- **THEN** stderr names `library/go/effective-go.md` and `--replace`, the exit code is 1, and the source and the stage folder are unchanged

#### Scenario: Nothing to replace
- **WHEN** `go/effective-go` does not exist and an agent runs `land` with `--replace`
- **THEN** the exit code is 1 and no file is written

### Requirement: The capture land keeps
`land` SHALL keep the staged text as `<root>/.bilbo/captures/<sha256 of capture.md>/capture.md`, with every other file of the stage folder except bilbo's own bookkeeping. A capture folder that already exists SHALL be kept as it is. `land` SHALL then append one line to the capture folder's `landed` file: the source's id, a tab, its `digest`, a tab and today's local date. A capture records a source when its `landed` file holds a line with that source's id and current `digest`.

#### Scenario: The capture is kept
- **WHEN** an agent lands `go/effective-go` from a stage
- **THEN** `<root>/.bilbo/captures/<hex>/capture.md` holds the staged text, `<hex>` is its SHA-256, and `landed` ends with the source's id, its digest and today's date

#### Scenario: The same text staged twice
- **WHEN** two stages hold the same text and both are landed, as two sources
- **THEN** one capture folder exists, and its `landed` file has two lines

#### Scenario: An existing capture is not rewritten
- **WHEN** the capture folder for a stage's text already exists, and an agent added a file `note.txt` to it by hand
- **THEN** after `land`, `capture.md` and `note.txt` have the same bytes and modification times as before, and only `landed` has a new line

### Requirement: A staged capture is fixed
`land` SHALL refuse a stage whose `capture.md` changed after `stage` wrote it, with exit 1 and a message saying to stage the file again, and SHALL write nothing. An unknown stage SHALL exit 1. A stage argument that is not a ULID SHALL be a usage error.

#### Scenario: An edited capture
- **WHEN** an agent edits `capture.md` in the stage folder, then runs `land`
- **THEN** stderr says the capture changed since it was staged, the exit code is 1, and no source, guide or capture folder is written

#### Scenario: An unknown stage
- **WHEN** an agent runs `bilbo library land 01M3EZ8NVEC2KJQNGK5DTK349R go/x --keep 1-2` and no such stage exists
- **THEN** stderr names the stage, and the exit code is 1

### Requirement: Land target refusals
A target that is not `<corpus>/<name>`, a corpus or source name that breaks the `library-store` name rules, a reserved corpus name, or the name `guide` SHALL be a usage error that writes nothing.

#### Scenario: The guide's name
- **WHEN** an agent runs `land <stage> go/guide --keep 1-10`
- **THEN** bilbo exits 2 and writes no file

#### Scenario: A reserved corpus
- **WHEN** an agent runs `land <stage> read/errors --keep 1-10`
- **THEN** bilbo exits 2 and writes no file

### Requirement: Duplicate origin warning
When another source in the library has the same `origin`, `land` SHALL still land, and SHALL print `bilbo: <origin> is also the origin of <corpus>/<name>` to stderr for each such source, where `<origin>` is the item without its quotes.

#### Scenario: The same page under a second name
- **WHEN** `go/effective-go` has `origin: "url: https://go.dev/doc/effective_go"` and an agent lands a stage with that origin as `go/effective-go-2026`
- **THEN** the exit code is 0 and stderr holds `bilbo: url: https://go.dev/doc/effective_go is also the origin of go/effective-go`

#### Scenario: A different origin gets no warning
- **WHEN** no other source has `origin: "url: https://go.dev/ref/spec"` and an agent lands a stage with that origin
- **THEN** stderr holds no line saying `is also the origin of`

### Requirement: No partial sources
`land` SHALL never leave a partly written file at a source's or a guide's path. Two `land` runs on one store SHALL take turns, so neither loses the other's guide entry. Two runs that land the same new `<corpus>/<name>` at the same time SHALL produce exactly one source: one run SHALL exit 0 and the other exit 1.

#### Scenario: Two lands into one corpus
- **WHEN** two `land` processes add `go/a` and `go/b` at the same moment
- **THEN** both exit 0, and the guide has both entries

#### Scenario: Two lands of one name
- **WHEN** two `land` processes add `go/errors` from two stages at the same moment
- **THEN** one exits 0, the other exits 1, and `errors.md` holds the id the successful run printed

### Requirement: Citation pre-check
Before `land --replace` writes a source whose digest changes, it SHALL check every citation of that source's id in the bodies of `<root>/notes/` against the old body and the new one, as the `citations` spec checks a draft without a plan. A citation degrades when its new verdict differs from its old one and is not `ok`. Citations whose verdict does not change, already failing ones included, do not block.

#### Scenario: A citation that keeps resolving
- **WHEN** a note cites `go/effective-go` with a quote that the new body still holds under the same anchor
- **THEN** `land --replace` lands the source and exits 0

#### Scenario: A citation that already failed
- **WHEN** a note's citation of `go/effective-go` is `quote_missing` against the old body and the new one
- **THEN** it does not block `land --replace`

#### Scenario: Citations in guides are not checked
- **WHEN** only `library/go/guide.md` cites the source
- **THEN** the pre-check finds no citation

### Requirement: Degraded citations block a replace
When any citation degrades and `--force` is not given, `land` SHALL exit 1 and write nothing, keeping the stage. Its stderr SHALL say how many citations would degrade and name `--force`, then give one line per citation: `notes/<file>:<line>: <old verdict> -> <new verdict>`. With `--force`, `land` SHALL replace the source and print the same per-citation lines to stderr. `--force` without `--replace` SHALL be a usage error.

#### Scenario: A quote the new text dropped
- **WHEN** `notes/gotcha-skill-frontmatter.md` cites `go/effective-go` on line 12 with an `ok` quote that the new body no longer holds, and an agent runs `land --replace` without `--force`
- **THEN** stderr holds a line naming 1 citation and `--force`, and the line `notes/gotcha-skill-frontmatter.md:12: ok -> quote_missing`; the exit code is 1; the source, the guide and the stage folder are unchanged, and no capture folder is written

#### Scenario: A heading renamed upstream
- **WHEN** a cited quote is still in the new body, under a heading whose text changed
- **THEN** its line reads `ok -> anchor_missing`, and `land` exits 1

#### Scenario: Replacing anyway
- **WHEN** the same agent runs the same `land --replace` with `--force`
- **THEN** the source is replaced, stderr holds `notes/gotcha-skill-frontmatter.md:12: ok -> quote_missing`, stdout holds the four lines of a successful land, and the exit code is 0

#### Scenario: Force alone
- **WHEN** an agent runs `land <stage> go/effective-go --keep 1-40 --force`
- **THEN** bilbo prints a message naming `--force` and `--replace` to stderr, exits 2 and writes no file
