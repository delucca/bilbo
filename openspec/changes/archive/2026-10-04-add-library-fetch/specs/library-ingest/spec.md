# Spec Delta

## MODIFIED Requirements

### Requirement: Stage a file
`bilbo library stage <file> --origin "<url|doc>: <value>" [--fetched <YYYY-MM-DD>] [--html]` SHALL read the file as UTF-8 text, drop a leading byte order mark, turn every CRLF and every lone CR into LF, add a final newline when it lacks one, and write the result as `capture.md` in a new folder `<state>/bilbo/staging/<stage>/`, where `<state>` is the state folder the usage message names and `<stage>` a fresh ULID. With `--html`, `capture.md` SHALL instead hold the file's text converted by the `library-fetch` HTML conversion rules, then normalized the same way, and the folder SHALL also hold the file's bytes as `raw`. `--fetched` SHALL default to today's local date. Staging SHALL change nothing under the store root.

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
