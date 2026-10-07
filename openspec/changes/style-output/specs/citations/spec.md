## MODIFIED Requirements

### Requirement: Verdicts
Each citation SHALL get one verdict: `ok` when the quote is in the anchored section, or, without an anchor, in a part of the body under no section; `quote_elsewhere` when it is elsewhere in the body, naming the deepest sections that hold it; `ambiguous` when the anchor matches several sections and one holds the quote; `quote_missing` when it is nowhere in the body, with the nearest passage as a hint, in the source's own words; `too_short` when it resolves with fewer than six words.

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

#### Scenario: A hint keeps the source's words
- **WHEN** the source says `the call returns SQLITE_BUSY at once` and the quote retypes it as `the call returns a busy error at once`
- **THEN** the verdict is `quote_missing` and the detail holds `SQLITE_BUSY`, not `SQLITEBUSY`

#### Scenario: Five words
- **WHEN** a five-word quote is in its section
- **THEN** the verdict is `too_short`
