# Spec Delta

## Purpose

`bilbo cite` checks that every citation in an agent's draft names a passage that exists, in a note or a source, and, given the read plans, that the agent read it. It replaces judgment about grounding with a verdict per citation and a coverage line counted from the read log.

## ADDED Requirements

### Requirement: Citation form
A citation SHALL be `bilbo:`, an id, optionally `#` and an anchor, one or more spaces or tabs, then a quote: text in straight double quotes, in which any straight double quote comes in pairs, or text in curly double quotes. A quote spans no blank line, and its closing quote is not followed by a letter or digit. The id is the 26 letters and digits right after `bilbo:`. Text that starts with `bilbo:` in any other shape is not a citation.

#### Scenario: A citation with an anchor
- **WHEN** a draft holds `bilbo:01M3EZ8NVEC2KJQNGK5DTK349R#Concurrency > Goroutines "A goroutine has a simple model: it is a function executing concurrently"`
- **THEN** it is one citation with that id, the anchor `Concurrency > Goroutines` and that quote

#### Scenario: Two citations on one line
- **WHEN** one line holds two citations, each with its own quote
- **THEN** both are checked, each on its own row

#### Scenario: Not a citation
- **WHEN** a draft holds `bilbo: no store at /tmp/x`
- **THEN** that text is not checked and gets no row

### Requirement: Resolving the id
The id SHALL resolve among the `id` of every file in `<root>/notes/` and `<root>/library/`, guides included. The cited text is that file's body, after its frontmatter. An id that no file has, or that two files share, gives the verdict `id_missing`.

#### Scenario: A source id
- **WHEN** the id is that of `library/go/effective-go.md`
- **THEN** the quote is searched in that source's body

#### Scenario: A note id
- **WHEN** the id is that of `notes/decision-release-tags.md`
- **THEN** the quote is searched in that note's body

#### Scenario: An unknown id
- **WHEN** no note or source has the id
- **THEN** the verdict is `id_missing`

### Requirement: Resolving the anchor
An anchor SHALL name a section of the cited file by the `library-store` spec's Outline and Anchors rules, the same rule `library show` and `library plan` use, for notes and sources alike. A section's text runs from the line after its heading to its end, so it holds its subsections. An anchor that matches no section gives `anchor_missing`; when the quote is in the body, the detail names the heading paths it sits under.

#### Scenario: A trailing part of the path
- **WHEN** a citation's anchor is `needless_return > What it does` in the clippy catalog
- **THEN** the quote is searched in that one section

#### Scenario: Markup in a heading
- **WHEN** a source has the heading `` ## The `Option` type `` and a citation's anchor is `The Option type`
- **THEN** the anchor resolves to that section

#### Scenario: The title is not an anchor
- **WHEN** a citation's anchor is the source's title, `Effective Go`
- **THEN** the verdict is `anchor_missing`, and when the quote is in the body the detail names where

### Requirement: Normalizing quotes
Before matching, both the quote and the cited text SHALL go through the `library-store` spec's Text normalization (NFKC, a few HTML entities decoded, quotes straightened, inline Markdown stripped, whitespace collapsed). On the quote alone, a line-number prefix (spaces, digits, then a tab or `→`) at the start of a line SHALL be dropped, and each run of three or more dots SHALL split it into fragments that must appear in that order.

#### Scenario: Markup and spacing differ
- **WHEN** the source says `the **zero value** is useful` across a line break and the quote is `the zero value is useful`
- **THEN** they match

#### Scenario: A pipe in a table cell
- **WHEN** a source's table cell holds `a &#124; b` and the quote retypes it as `a | b`
- **THEN** they match

#### Scenario: A quote copied from library read
- **WHEN** the quote holds `820	A goroutine has a simple model` copied with its line number
- **THEN** the line number is dropped and the rest matches

#### Scenario: Fragments in order
- **WHEN** the quote is `goroutines are multiplexed ... onto multiple OS threads`
- **THEN** it matches when both fragments occur in that order in the section, and does not when they occur in the other order

### Requirement: Verdicts
Each citation SHALL get one verdict: `ok` when the quote is in the anchored section, or, without an anchor, in a part of the body under no section; `quote_elsewhere` when it is elsewhere in the body, naming the deepest sections that hold it; `ambiguous` when the anchor matches several sections and one holds the quote; `quote_missing` when it is nowhere in the body, with the nearest passage as a hint; `too_short` when it resolves with fewer than six words.

#### Scenario: The quote under its heading
- **WHEN** the quote sits in the section the anchor names, or in one of its subsections
- **THEN** the verdict is `ok`

#### Scenario: A wrong anchor
- **WHEN** the quote sits under `Concurrency > Channels` and the anchor is `Goroutines`
- **THEN** the verdict is `quote_elsewhere` and the detail names `Concurrency > Channels`

#### Scenario: No anchor in a file with sections
- **WHEN** a citation has no anchor and its quote sits under `Errors > Wrapping`
- **THEN** the verdict is `quote_elsewhere` and the detail names `Errors > Wrapping`

#### Scenario: A headingless source
- **WHEN** a source has no heading below its title and a citation without an anchor quotes its text
- **THEN** the verdict is `ok`

#### Scenario: A repeated heading
- **WHEN** the anchor is `What it does`, which many lints share, and one of them holds the quote
- **THEN** the verdict is `ambiguous` and the detail names that section's full heading path

#### Scenario: A retyped quote
- **WHEN** the quote is not in the body after normalization
- **THEN** the verdict is `quote_missing`, and the detail holds the passage of the body that shares the most words with it

#### Scenario: Five words
- **WHEN** a five-word quote is in its section
- **THEN** the verdict is `too_short`

### Requirement: Check a draft
`bilbo cite [--plan <plan>]... [<file> | -]` SHALL read the draft from the file, or from stdin when no file or `-` is given, and print one row per citation in the order they appear: its line in the draft, the verdict, the id with its anchor as written, the cited file's absolute path or `-`, and the detail, joined by tabs. Then it SHALL print `citations: <n> checked, <k> ok`. It SHALL read no settings and write no file.

#### Scenario: Two citations
- **WHEN** a draft on stdin holds an `ok` citation on line 3 and a `quote_elsewhere` one on line 8
- **THEN** stdout holds a row starting `3`, a tab and `ok`, a row starting `8`, a tab and `quote_elsewhere`, then `citations: 2 checked, 1 ok`

#### Scenario: No citations
- **WHEN** the draft holds no citation
- **THEN** stdout is `citations: 0 checked, 0 ok`, stderr is `bilbo: no citations found`, and the exit code is 0

#### Scenario: A citation without a quote
- **WHEN** line 5 holds `bilbo:01M3EZ8NVEC2KJQNGK5DTK349R#Concurrency` with no quoted text after it
- **THEN** stderr holds a line saying the citation on line 5 has no quote and was not checked

#### Scenario: The old form
- **WHEN** a draft holds `note: /Users/a/Notebooks/x/library/go.md#Errors "some quoted words here"`
- **THEN** stderr holds a line naming its line and saying the old `note:` form is not checked, and the same holds for `note: ~/...` and for a relative `note: ./...`

### Requirement: Exit code
`cite` SHALL exit 1 when any verdict is `quote_missing`, `anchor_missing`, `id_missing` or `unread`, and 0 otherwise. A file that is missing or unreadable SHALL exit 1 with the reason. An unknown option or a second file SHALL be a usage error. When neither `<root>/notes/` nor `<root>/library/` exists, `cite` SHALL exit 1 with `bilbo: no store at <root>`. With `--plan`, when neither `XDG_STATE_HOME` nor `HOME` is an absolute path, `cite` SHALL exit 2 naming them.

#### Scenario: Warnings pass
- **WHEN** the verdicts are `ok`, `too_short`, `quote_elsewhere` and `ambiguous`
- **THEN** the exit code is 0

#### Scenario: A failure fails
- **WHEN** one verdict of ten is `quote_missing`
- **THEN** every row is still printed and the exit code is 1

#### Scenario: No state folder
- **WHEN** an agent runs `bilbo cite --plan <plan>` with `XDG_STATE_HOME` unset and `HOME` relative
- **THEN** stderr names `XDG_STATE_HOME` and `HOME`, and the exit code is 2

#### Scenario: A missing draft
- **WHEN** an agent runs `bilbo cite /tmp/nope.md` and that file does not exist
- **THEN** stderr names `/tmp/nope.md` and the exit code is 1

### Requirement: Unread citations
With `--plan <plan>`, given once or more, a citation of a source SHALL get `unread`, instead of `ok`, `too_short`, `quote_elsewhere` or `ambiguous`, when no match of its quote lies wholly in lines that the read log of one of those plans records as read, for the `digest` the source has now. The detail SHALL name the lines and their slice, or say the source is in no plan or changed since its plan. Citations of notes and guides are never `unread`: neither can be planned.

#### Scenario: A quote from a slice that was read
- **WHEN** slice 3 of the plan was read and the quote sits in its lines
- **THEN** the verdict is `ok`

#### Scenario: A quote from a slice nobody read
- **WHEN** slice 8 holds the quote and was never read
- **THEN** the verdict is `unread`, the detail names the lines and slice 8, and the exit code is 1

#### Scenario: A source outside the plan
- **WHEN** the cited source is in no pick of the plan
- **THEN** the verdict is `unread` and the detail says the source is in no plan

#### Scenario: A note under a plan
- **WHEN** a citation of a note resolves and `--plan` is given
- **THEN** the verdict is `ok`

#### Scenario: A guide under a plan
- **WHEN** a citation of `library/go/guide.md` resolves and `--plan` is given
- **THEN** the verdict is `ok`

#### Scenario: An unknown plan
- **WHEN** `--plan` names no plan in `<state>/bilbo/plans/`
- **THEN** stderr names the plan and the exit code is 1

### Requirement: Coverage lines
With `--plan`, after the citations line, `cite` SHALL print for each plan `coverage: plan <plan>: read <r> of <s> slices (<t> of <T> tokens); not read: ` and the unread line runs as `<corpus>/<name> lines <a>-<b> (slice <i>)`, or `(slices <i>-<j>)` for consecutive slices, joined by `, `, or `none`. Then `picked: plan <plan>: ` and, per corpus, `<corpus> <k> of <n> sources (<picks>)`, joined by `; `, where `<n>` counts the corpus's sources now.

#### Scenario: A partly read plan
- **WHEN** a plan has 9 slices and slices 8 and 9 of `go/effective-go`, lines 1200 to 1500, were not read
- **THEN** a line reads `coverage: plan <plan>: read 7 of 9 slices (<t> of <T> tokens); not read: go/effective-go lines 1200-1500 (slices 8-9)`

#### Scenario: Everything read
- **WHEN** every slice of the plan was read
- **THEN** its coverage line ends with `not read: none`

#### Scenario: The picks against the corpus
- **WHEN** the plan picked `go/effective-go` and `go/errors#Wrapping`, and the `go` corpus holds 14 sources
- **THEN** a line reads `picked: plan <plan>: go 2 of 14 sources (effective-go, errors#Wrapping)`

#### Scenario: No plan, no coverage
- **WHEN** `cite` runs without `--plan`
- **THEN** stdout has no `coverage:` and no `picked:` line
