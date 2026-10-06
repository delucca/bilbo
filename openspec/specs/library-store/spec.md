# library-store Specification

## Purpose
Where bilbo keeps sources and the exact shape of a source file and of a corpus guide. `bilbo library land` writes sources, agents edit guides, and `bilbo check` and every library verb rely on this contract.

## Requirements

### Requirement: Library layout
Sources SHALL live in `<root>/library/<corpus>/<name>.md`, one file per source whatever its size. `<root>/library/` SHALL hold only corpus folders, and a corpus folder SHALL hold its `guide.md` and its sources, with no subfolder and no other file. An entry whose name starts with `.` SHALL be ignored, in `<root>/library/` and in every corpus.

#### Scenario: A source in a corpus
- **WHEN** `<root>/library/go/effective-go.md` exists
- **THEN** it is the source `effective-go` of the corpus `go`

#### Scenario: A hidden entry is ignored
- **WHEN** `<root>/library/.lock` and `<root>/library/go/.DS_Store` exist
- **THEN** bilbo treats neither as a corpus, a source or a problem

#### Scenario: A subfolder in a corpus is not allowed
- **WHEN** `<root>/library/go/effective-go/` exists
- **THEN** the store is invalid at that entry

#### Scenario: A loose file in the library is not allowed
- **WHEN** `<root>/library/effective-go.md` exists
- **THEN** the store is invalid at that entry

### Requirement: Corpus and source names
A corpus name and a source name SHALL follow the topic grammar of the `note-store` spec: segments of `[a-z0-9]+` joined by single hyphens. The corpus names `show`, `stage`, `land`, `plan` and `read` SHALL be invalid, because `bilbo library <corpus>` shares its argument with the subcommands. `guide.md` is the guide, so no source is named `guide`.

#### Scenario: Valid names
- **WHEN** `<root>/library/software-architecture/action-domain-responder.md` exists
- **THEN** both names are valid

#### Scenario: A bad corpus name
- **WHEN** `<root>/library/Go/` or `<root>/library/go--style/` exists
- **THEN** the store is invalid at that entry

#### Scenario: A reserved corpus name
- **WHEN** `<root>/library/plan/` exists
- **THEN** the store is invalid at that entry, and the problem says `plan` is reserved

#### Scenario: A bad source name
- **WHEN** `<root>/library/go/Effective_Go.md` or `<root>/library/go/effective-go.txt` exists
- **THEN** the store is invalid at that entry

### Requirement: Source frontmatter
A source SHALL open with a line holding exactly `---`, then its keys, then a second line holding exactly `---`. The keys SHALL be `id`, `fetched`, `origin` and `digest`, all required, and optionally `kept` and `capture`, in any order, each at most once, one per line as `<key>: <value>`. No other key is allowed. `id` SHALL follow the Note id rule of the `note-store` spec.

#### Scenario: A full frontmatter
- **WHEN** a source opens with `---`, `id: 01M3EZ8NVEC2KJQNGK5DTK349R`, `fetched: 2026-08-23`, `origin: "url: https://go.dev/doc/effective_go"`, `digest: sha256:` and 64 lowercase hex digits that match its body, `kept: 12-1904,1910-1950`, `capture: external` and `---`
- **THEN** the frontmatter is valid

#### Scenario: Note keys are not source keys
- **WHEN** a source's frontmatter also holds `created: 2026-09-26T10:12-03:00`, `scope: go` or a `sources:` list
- **THEN** the source is invalid, and the problem names that key

#### Scenario: A required key is missing
- **WHEN** a source's frontmatter has no `digest` line
- **THEN** the source is invalid, and the problem names `digest`

### Requirement: Fetched and origin
`fetched` SHALL be a real date written `YYYY-MM-DD`: the day the text was taken from its origin. `origin` SHALL be exactly one double-quoted `<type>: <value>`, the type `url` or `doc` and the value not empty, holding no `"` and no `\`.

#### Scenario: A valid origin
- **WHEN** a source has `fetched: 2026-08-23` and `origin: "doc: The Go Programming Language, chapter 8"`
- **THEN** both are valid

#### Scenario: A bad date
- **WHEN** a source has `fetched: 2026-02-30` or `fetched: 2026-08-23T10:00-03:00`
- **THEN** the source is invalid, and the problem names `fetched`

#### Scenario: A bad origin
- **WHEN** a source has `origin: https://go.dev`, `origin: "code: src/main.rs"` or `origin: "url: "`
- **THEN** the source is invalid, and the problem names `origin`

### Requirement: Kept ranges and capture label
`kept`, when present, SHALL be 1-based inclusive line ranges `<a>-<b>` joined by commas, with `a` not above `b` and each range starting after the previous one ends. It names the lines of the capture that the body holds, and is absent when the whole capture was kept. `capture`, when present, SHALL be `external`: text bilbo did not fetch. It is absent for text bilbo fetched itself.

#### Scenario: Valid ranges
- **WHEN** a source has `kept: 3-120,130-130,140-200`
- **THEN** `kept` is valid

#### Scenario: Unordered or overlapping ranges
- **WHEN** a source has `kept: 130-200,3-120`, `kept: 3-120,100-200`, `kept: 0-10` or `kept: 20-10`
- **THEN** the source is invalid, and the problem names `kept`

#### Scenario: An external capture
- **WHEN** a source has `capture: external`
- **THEN** `capture` is valid

#### Scenario: An unknown capture label
- **WHEN** a source has `capture: webfetch`
- **THEN** the source is invalid, and the problem names `capture` and says it is not `external`

### Requirement: Body digest
`digest` SHALL be `sha256:` followed by the SHA-256, in lowercase hex, of every byte after the line that closes the frontmatter. A source whose body does not hash to its `digest` is invalid. Only `bilbo library land` writes a source file.

#### Scenario: A landed source
- **WHEN** `bilbo library land` wrote a source and nothing changed it since
- **THEN** its body hashes to its `digest`

#### Scenario: A hand edit
- **WHEN** an agent changes one character in a source's body, or appends a newline to it
- **THEN** the source is invalid, and the problem names `digest`

### Requirement: Source title
A source's body SHALL open with a level-1 heading, a line starting with `# `, and SHALL hold no other level-1 heading outside fenced code blocks.

#### Scenario: One title
- **WHEN** a source's body starts with `# Effective Go` and holds level-2 headings and a fenced `# comment` line
- **THEN** the title is valid

#### Scenario: A title that is not first
- **WHEN** a source's body starts with a blank line or a paragraph before `# Effective Go`
- **THEN** the source is invalid, and the problem names the title

#### Scenario: Two titles
- **WHEN** a source's body holds two lines starting with `# ` outside fences
- **THEN** the source is invalid, and the problem names the title

### Requirement: The guide
Each corpus SHALL hold `guide.md`, the only file in a corpus that agents edit. It SHALL follow the `note-store` rules for frontmatter delimiters, `id`, `created` and the title, with `id` and `created` as its only keys. After the title come a lead paragraph on what the corpus grounds and who reads it, then the entries.

#### Scenario: A valid guide
- **WHEN** `<root>/library/go/guide.md` holds `---`, `id: 01M3EZ8NBEVNHZRTQ6T60171J2`, `created: 2026-09-26T13:16-03:00`, `---`, `# Go`, a lead and one entry per source
- **THEN** the guide is valid

#### Scenario: A missing guide
- **WHEN** `<root>/library/go/` holds sources and no `guide.md`
- **THEN** the store is invalid at `library/go/guide.md`

#### Scenario: A guide with a sources list
- **WHEN** a guide's frontmatter holds a `sources:` list
- **THEN** the guide is invalid, and the problem names `sources`

### Requirement: Guide entries
Every level-2 heading outside fenced code blocks in a guide SHALL be an entry, headed `## <name>` with the name of a source in the same corpus, the file name without `.md`. Every source SHALL have exactly one entry, and every entry SHALL name a source. The prose under an entry is authored. Derived facts such as sizes, token counts, `fetched`, heading counts or the catalog mark SHALL NOT be required in a guide, and bilbo never writes them there.

#### Scenario: Each source has its entry
- **WHEN** `library/go/` holds `effective-go.md` and `uber-go-style-guide.md`, and its guide has `## effective-go` and `## uber-go-style-guide`, each with prose
- **THEN** the corpus is valid on this rule

#### Scenario: A source with no entry
- **WHEN** `library/go/effective-go.md` exists and the guide has no `## effective-go`
- **THEN** the store is invalid at `library/go/effective-go.md`

#### Scenario: An entry with no source
- **WHEN** the guide has `## effective-go` or `## Reading order` and no source by that name exists in the corpus
- **THEN** the store is invalid at the guide, on that heading's line

#### Scenario: Two entries for one source
- **WHEN** the guide has `## effective-go` twice
- **THEN** the store is invalid at the guide, on the second heading's line

### Requirement: Stub and stale lines
A guide SHALL hold no line that is exactly `TODO: describe this corpus.` or `TODO: describe this source.`, and no line starting with `stale: re-ingested `. `bilbo library land` writes them to ask for prose; an agent writes or revises the prose, then removes the line.

#### Scenario: A stub entry
- **WHEN** the entry `## effective-go` holds the line `TODO: describe this source.`
- **THEN** the store is invalid at the guide, on that line, and the problem names the entry

#### Scenario: A stale entry
- **WHEN** the entry `## effective-go` holds the line `stale: re-ingested 2026-10-03; re-read the source and revise this entry.`
- **THEN** the store is invalid at the guide, on that line, and the problem names the entry

#### Scenario: A stub lead
- **WHEN** the guide's lead is the line `TODO: describe this corpus.`
- **THEN** the store is invalid at the guide, on that line

#### Scenario: Revised prose passes
- **WHEN** an agent replaces the stub with two sentences and removes the `TODO` line
- **THEN** the guide is valid on this rule

### Requirement: Ids across notes and library
Every `id` in `<root>/notes/` and in `<root>/library/`, guides included, SHALL be unique across both folders. A citation can then name any note or source by its id alone.

#### Scenario: A source shares a note's id
- **WHEN** `notes/plan-a.md` and `library/go/effective-go.md` have the same `id`
- **THEN** the store is invalid at both files

#### Scenario: A guide shares a source's id
- **WHEN** `library/go/guide.md` and `library/rust/clippy-lints.md` have the same `id`
- **THEN** the store is invalid at both files

### Requirement: Outline
A source's sections SHALL be its headings below the title, outside fenced code blocks, at any level, recognized as the `note-recall` spec's Passages requirement recognizes headings. A section starts at its heading line and runs to the line before the next heading of the same or a higher level, or to the file's last line. Its heading path is the texts of its enclosing headings below the title and its own, joined by ` > `. Line numbers are the file's physical lines, from 1.

#### Scenario: A nested heading path
- **WHEN** `### Goroutines` sits under `## Concurrency` in the source titled `# Effective Go`
- **THEN** that section's heading path is `Concurrency > Goroutines`, and the `Concurrency` section runs past it to the next level-2 heading

#### Scenario: A heading in a code fence opens no section
- **WHEN** a fenced block in a source holds the line `## not a heading`
- **THEN** that line starts no section

#### Scenario: A headingless source
- **WHEN** a source's body holds its title and paragraphs, and no other heading
- **THEN** the source has no sections

### Requirement: Derived sizes
A source's size SHALL be the number of bytes of its body, and its tokens that number divided by 2.5, rounded up. A section's size SHALL be the bytes of its lines, each with its newline. A size in KB SHALL be bytes divided by 1,000, rounded up. A corpus's bytes and tokens SHALL be the sums of its sources' bytes and tokens, and its KB SHALL come from its bytes.

#### Scenario: Round numbers
- **WHEN** a source's body is 1,000 bytes
- **THEN** it is 400 tokens and 1 KB

#### Scenario: Rounding up
- **WHEN** a source's body is 1,001 bytes
- **THEN** it is 401 tokens and 2 KB

#### Scenario: A corpus's KB is not a sum of rounded KBs
- **WHEN** a corpus holds two sources of 1,001 bytes each
- **THEN** the corpus is 2,002 bytes, 802 tokens and 3 KB, not the 4 KB its sources' rounded sizes would add up to

### Requirement: Catalog
A source SHALL be a catalog when its body is over 55,000 bytes and more than 40 sections sit at or above its cut level: the shallowest heading level, from 2 to 6, at which at least two sections sit at that level or above. Whether a source is a catalog is derived when needed and never written into any file.

#### Scenario: A lint catalog
- **WHEN** a source's body is 891,000 bytes and holds 849 level-2 sections, each with level-3 sections
- **THEN** its cut level is 2, 849 sections sit there, and the source is a catalog

#### Scenario: A book is not a catalog
- **WHEN** a source's body is 102,000 bytes and holds 16 level-2 sections with 43 level-3 sections among them
- **THEN** its cut level is 2, 16 sections sit there, and the source is not a catalog

#### Scenario: One chapter over many entries
- **WHEN** a source's body is 80,000 bytes and holds one level-2 section with 60 level-3 sections inside it
- **THEN** its cut level is 3, 61 sections sit at level 3 or above, and the source is a catalog

#### Scenario: A short source with many sections
- **WHEN** a source's body is 30,000 bytes and holds 300 level-2 sections
- **THEN** it is not a catalog

### Requirement: Anchors
An anchor SHALL name a section by its heading path, or by any trailing part of it, parts joined by ` > `. It matches a section when its parts equal the last parts of that section's heading path, each part and each heading compared after the Text normalization, with case. This one rule serves every verb that takes an anchor. One matching section resolves the anchor; several make it ambiguous; none make it missing.

#### Scenario: A trailing part resolves
- **WHEN** a source has the sections `needless_return > What it does` and `needless_range_loop > What it does`, and the anchor is `needless_return > What it does`
- **THEN** the anchor resolves to the first of them

#### Scenario: A bare heading that repeats is ambiguous
- **WHEN** the anchor is `What it does` in that source
- **THEN** the anchor is ambiguous

#### Scenario: Case counts
- **WHEN** the anchor is `what it does` in that source
- **THEN** the anchor is missing

#### Scenario: Markup in a heading
- **WHEN** a source has the heading `` ## The `Option` type `` and an agent runs `bilbo library show 'rust/book#The Option type'`
- **THEN** the anchor resolves to that section, as the anchor `` The `Option` type `` does

#### Scenario: Different words stay different
- **WHEN** the anchor is `The Options type` in that source
- **THEN** the anchor is missing

### Requirement: Captures
The text a source was cut from SHALL be kept as `<root>/.bilbo/captures/<sha256>/capture.md`, the folder named by the SHA-256, in lowercase hex, of that file. Captures are local evidence: a source SHALL be valid whether or not its capture exists, and `bilbo check` SHALL NOT read `<root>/.bilbo/captures/`.

#### Scenario: A source without its capture
- **WHEN** a store holds a valid source and `<root>/.bilbo/captures/` is missing, as on a fresh copy of the library
- **THEN** the source is valid

#### Scenario: Captures are not checked
- **WHEN** `<root>/.bilbo/captures/` holds a folder whose name does not match its `capture.md`, and a folder with no `capture.md`
- **THEN** `bilbo check` reports nothing about either

### Requirement: Text normalization
Where a spec compares text after the Text normalization, bilbo SHALL apply Unicode NFKC, decode `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;` and `&#124;`, make curly quotes straight, reduce links and images to their text and autolinks to their target, drop backslash escapes, `*`, `_`, backticks and `~~`, and collapse runs of whitespace to one space, trimmed. Case is kept.

#### Scenario: An entity in a table cell
- **WHEN** a source's table cell holds `a &#124; b` and the compared text is `a | b`
- **THEN** both normalize to `a | b`

#### Scenario: Emphasis and spacing
- **WHEN** one text is `the **zero  value**` and the other is `the zero value`
- **THEN** both normalize to `the zero value`

#### Scenario: Case is not folded
- **WHEN** one text is `Option` and the other is `option`
- **THEN** they normalize to different text
